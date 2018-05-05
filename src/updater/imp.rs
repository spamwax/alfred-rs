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
                avail_version: None,
                last_check: Cell::new(None),
                rx: RefCell::new(None),
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
        let current_version = self.current_version().clone();

        thread::Builder::new().spawn(move || {
            let mut talk_to_mother = || -> Result<(), Error> {
                let (update_avail, v) =
                    releaser.latest_version().map(|v| (current_version < v, v))?;
                let payload = if update_avail {
                    let url = releaser.downloadable_url()?;
                    let info = UpdateInfo {
                        avail_version: v,
                        downloadable_url: url,
                    };
                    Some(info)
                } else {
                    None
                };
                tx.send(Ok(payload.clone()))?;
                Self::write_last_check_status(&p, payload)?;
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
        version: Option<UpdateInfo>,
    ) -> Result<(), Error> {
        File::create(p)
            .and_then(|fp| {
                let buf_writer = BufWriter::with_capacity(128, fp);
                serde_json::to_writer(buf_writer, &version)?;
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
}

pub(super) fn default_interval() -> i64 {
    UPDATE_INTERVAL
}
