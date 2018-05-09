use super::*;
use std::sync::mpsc;
use Updater;

/// Default update interval duration 24 hr
const UPDATE_INTERVAL: i64 = 24 * 60 * 60;

pub(super) const LATEST_UPDATE_INFO_CACHE_FN: &str = "last_check_status.json";
pub(super) const LATEST_UPDATE_INFO_CACHE_FN_ASYNC: &str = "last_check_status_async.json";

impl<T> Updater<T>
where
    T: Releaser + Send + 'static,
{
    pub(super) fn load_or_new(r: T) -> Result<Self, Error> {
        if let Ok(mut saved_state) = Self::load() {
            // Overwrite saved state's current_version if the version that
            // may have been set through env. variable is semantically
            // newer than version saved in state.
            let env_ver = env::workflow_version().and_then(|v| Version::parse(&v).ok());
            if let Some(v) = env_ver {
                if v > saved_state.current_version {
                    saved_state.current_version = v;
                }
            }
            Ok(Updater {
                state: saved_state,
                releaser: RefCell::new(r),
            })
        } else {
            let current_version = env::workflow_version()
                .map_or_else(|| Ok(Version::from((0, 0, 0))), |v| Version::parse(&v))?;
            let state = UpdaterState {
                current_version,
                avail_version: RefCell::new(None),
                last_check: Cell::new(None),
                worker_state: RefCell::new(None),
                update_interval: UPDATE_INTERVAL,
            };
            let updater = Updater {
                state,
                releaser: RefCell::new(r),
            };
            updater.save()?;
            Ok(updater)
        }
    }

    pub(super) fn current_version(&self) -> &Version {
        &self.state.current_version
    }

    pub(super) fn last_check(&self) -> Option<DateTime<Utc>> {
        self.state.last_check.get()
    }

    pub(super) fn set_last_check(&self, t: DateTime<Utc>) {
        self.state.last_check.set(Some(t));
    }

    pub(super) fn update_interval(&self) -> i64 {
        self.state.update_interval
    }

    pub(super) fn set_update_interval(&mut self, t: i64) {
        self.state.update_interval = t;
    }

    fn load() -> Result<UpdaterState, Error> {
        Self::build_data_fn().and_then(|data_file_path| {
            if data_file_path.exists() {
                Ok(File::open(data_file_path).and_then(|fp| {
                    let buf_reader = BufReader::with_capacity(128, fp);
                    Ok(serde_json::from_reader(buf_reader)?)
                })?)
            } else {
                Err(err_msg("missing updater data file"))
            }
        })
    }

    pub(super) fn save(&self) -> Result<(), Error> {
        let data_file_path = Self::build_data_fn().and_then(|data_file_path| {
            create_dir_all(data_file_path.parent().unwrap())?;
            Ok(data_file_path)
        })?;
        File::create(&data_file_path)
            .and_then(|fp| {
                let buf_writer = BufWriter::with_capacity(128, fp);
                serde_json::to_writer(buf_writer, &self.state)?;
                Ok(())
            })
            .or_else(|e| {
                let _ = remove_file(data_file_path);
                Err(e.into())
            })
    }

    pub(super) fn start_releaser_worker(
        &self,
        tx: mpsc::Sender<Result<Option<UpdateInfo>, Error>>,
        p: PathBuf,
    ) -> Result<(), Error> {
        use std::thread;

        let mut releaser = (*self.releaser.borrow()).clone();

        thread::Builder::new().spawn(move || {
            let mut talk_to_mother = || -> Result<(), Error> {
                let v = releaser.latest_version()?;
                let payload = {
                    let url = releaser.downloadable_url()?;
                    let info = UpdateInfo {
                        avail_version: v,
                        downloadable_url: url,
                    };
                    Some(info)
                };
                Self::write_last_check_status(&p, &payload)?;
                tx.send(Ok(payload))?;
                Ok(())
            };

            let outcome = talk_to_mother();

            if let Err(error) = outcome {
                print!("worker outcome is error: {:?}", error);
                tx.send(Err(error))
                    .expect("could not send error from thread");
            }
        })?;
        Ok(())
    }

    // write version of latest avail. release (if any) to a cache file
    pub(super) fn write_last_check_status(
        p: &PathBuf,
        updater_info: &Option<UpdateInfo>,
    ) -> Result<(), Error> {
        File::create(p)
            .and_then(|fp| {
                let buf_writer = BufWriter::with_capacity(128, fp);
                serde_json::to_writer(buf_writer, updater_info)?;
                Ok(())
            })
            .or_else(|e| {
                let _ = remove_file(p);
                Err(e)
            })?;
        Ok(())
    }

    // read version of latest avail. release (if any) from a cache file
    pub(super) fn read_last_check_status(p: &PathBuf) -> Result<Option<UpdateInfo>, Error> {
        Ok(File::open(p).and_then(|fp| {
            let buf_reader = BufReader::with_capacity(128, fp);
            let v = serde_json::from_reader(buf_reader)?;
            Ok(v)
        })?)
    }

    pub(super) fn build_data_fn() -> Result<PathBuf, Error> {
        let workflow_name = env::workflow_name()
            .unwrap_or_else(|| "YouForgotTo/フ:NameYourOwnWork}flowッ".to_string())
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>();

        env::workflow_data()
            .ok_or_else(|| err_msg("missing env variable for data dir"))
            .and_then(|mut data_path| {
                env::workflow_uid()
                    .ok_or_else(|| err_msg("missing env variable for uid"))
                    .and_then(|ref uid| {
                        let filename = [uid, "-", workflow_name.as_str(), "-updater.json"].concat();
                        data_path.push(filename);

                        Ok(data_path)
                    })
            })
    }

    /// Checks if a new update is available (blocking).
    ///
    /// This method will fetch the latest release information from repository
    /// and compare it to the current release of the workflow. The repository should
    /// tag each release according to semantic version scheme for this to work.
    ///
    /// The method **will** make a network call to fetch metadata of releases *only if* UPDATE_INTERVAL
    /// seconds has passed since the last network call.
    ///
    /// All calls, which happen before the UPDATE_INTERVAL seconds, will use a local cache
    /// to report availability of a release without blocking the main thread.
    ///
    /// For `Updater`s talking to `github.com`, this method will only fetch a small metadata file to extract
    /// the version info of the latest release.
    ///
    /// # Note
    ///
    /// Since this method blocks the current thread until a response is received from remote server,
    /// workflow authors should consider scenarios where network connection is poor and the block can
    /// take a long time (>1 second), and devise their workflow around it. An alternative to
    /// this method is the non-blocking [`update_ready_async()`].
    ///
    /// # Example
    ///
    /// ```no_run
    /// # extern crate alfred;
    /// # extern crate failure;
    /// use alfred::Updater;
    ///
    /// # use failure::Error;
    /// # use std::io;
    /// # fn main() {
    /// let updater =
    ///     Updater::gh("kballard/alfred-rs").expect("cannot initiate Updater");
    ///
    /// // The very first call to `update_ready()` will return `false`
    /// // since it's assumed that user has just downloaded the workflow.
    /// assert_eq!(false, updater.update_ready().unwrap());
    ///
    /// // Above will save the state of `Updater` in workflow's data folder.
    /// // Depending on how long has elapsed since first run, consequent calls
    /// // to `update_ready()` may return false if it has been less than
    /// // the interval set for checking (defaults to 24 hours).
    ///
    /// // However in subsequent runs, when the checking interval period has elapsed
    /// // and there actually exists a new release, then `update_ready()` will return true.
    /// assert_eq!(true, updater.update_ready().unwrap());
    ///
    /// # }
    /// ```
    ///
    /// # Errors
    /// Checking for update can fail if network error, file error or Alfred environment variable
    /// errors happen.
    ///
    /// [`update_ready_async()`]: struct.Updater.html#method.update_ready_async
    pub(super) fn update_ready_sync(&self) -> Result<bool, Error> {
        // A None value for last_check indicates that workflow is being run for first time.
        // Thus we update last_check to now and just save the updater state without asking
        // Releaser to do a remote call/check for us since we assume that user just downloaded
        // the workflow.
        use self::imp::LATEST_UPDATE_INFO_CACHE_FN;

        // file for status of last update check
        let p = Self::build_data_fn()?.with_file_name(LATEST_UPDATE_INFO_CACHE_FN);

        // make a network call to see if a newer version is avail.
        // save the result of call to cache file.
        let ask_releaser_for_update = || -> Result<bool, Error> {
            let (update_avail, v) = self.releaser
                .borrow_mut()
                .latest_version()
                .map(|v| (*self.current_version() < v, v))?;

            let payload = {
                let url = self.releaser.borrow().downloadable_url()?;
                let info = UpdateInfo {
                    avail_version: v,
                    downloadable_url: url,
                };
                Some(info)
            };
            Self::write_last_check_status(&p, &payload)?;
            *self.state.avail_version.borrow_mut() = payload;

            self.set_last_check(Utc::now());
            self.save()?;
            Ok(update_avail)
        };

        // if first time checking, just update the updater's timestamp, no network call
        if self.last_check().is_none() {
            self.set_last_check(Utc::now());
            self.save()?;
            Ok(false)
        } else if self.due_to_check() {
            // it's time to talk to remote server
            ask_releaser_for_update()
        } else {
            Self::read_last_check_status(&p)
                .map(|last_check_status| {
                    last_check_status
                        .map(|last_update_info| {
                            if self.current_version() < &last_update_info.avail_version {
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false)
                })
                .or(Ok(false))
        }
    }

    pub(super) fn update_ready_async_(&self, try_flag: bool) -> Result<bool, Error> {
        self.state
            .worker_state
            .borrow()
            .as_ref()
            .ok_or(err_msg("you need to use init() metheod first."))
            .and_then(|mpsc| {
                if mpsc.recvd_payload.borrow().is_none() {
                    // No payload received yet, try to talk to worker thread
                    mpsc.rx
                        .borrow()
                        .as_ref()
                        .ok_or(err_msg("you need to use init() correctly!"))
                        .and_then(|rx| {
                            let rr = if try_flag {
                                // don't block while trying to receive
                                rx.try_recv().map_err(|e| err_msg(format!("{}", e)))
                            } else {
                                // block while waiting to receive
                                rx.recv().map_err(|e| err_msg(format!("{}", e)))
                            };
                            rr.and_then(|msg| {
                                let msg_status = msg.map(|update_info| {
                                    // received good messag, update cache for received payload
                                    *self.state.avail_version.borrow_mut() = update_info.clone();
                                    *mpsc.recvd_payload.borrow_mut() =
                                        Some(Ok(update_info.clone()));
                                });
                                // save state regardless of content of msg
                                self.set_last_check(Utc::now());
                                self.save()?;
                                Ok(msg_status?)
                            })
                        })?;
                }
                Ok(())
            })?;
        Ok(self.state
            .avail_version
            .borrow()
            .as_ref()
            .map(|release| {
                if self.current_version() < &release.avail_version {
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false))
    }

    #[allow(dead_code)]
    pub(super) fn update_ready_async__(&self) -> Result<bool, Error> {
        let worker_state = self.state.worker_state.borrow();
        if worker_state.is_none() {
            panic!("you need to use init first")
        };

        let mpsc = worker_state.as_ref().expect("no worker_state");
        if mpsc.recvd_payload.borrow().is_none() {
            let rx_option = mpsc.rx.borrow();
            let rx = rx_option.as_ref().unwrap();
            let rr = rx.recv();
            if rr.is_ok() {
                let msg = rr.as_ref().unwrap();
                if msg.is_ok() {
                    let update_info = msg.as_ref().unwrap();
                    *self.state.avail_version.borrow_mut() = update_info.clone();
                    *mpsc.recvd_payload.borrow_mut() = Some(Ok(update_info.clone()));
                } else {
                    return Err(err_msg(format!("{:?}", msg.as_ref().unwrap_err())));
                }
                self.set_last_check(Utc::now());
                self.save()?;
            } else {
                eprintln!("{:?}", rr);
                return Err(err_msg(format!("{:?}", rr)));
            }
        }
        if self.state.avail_version.borrow().is_some()
            && self.current_version()
                < &self.state
                    .avail_version
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .avail_version
        {
            return Ok(true);
        } else {
            return Ok(false);
        }
    }
}

pub(super) fn default_interval() -> i64 {
    UPDATE_INTERVAL
}
