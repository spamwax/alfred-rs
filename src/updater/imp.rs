use super::Error;
use super::Releaser;
use super::*;
use std::sync::mpsc;
use Updater;

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
                last_check: Cell::new(None),
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
        Self::build_data_fn()
            .and_then(|data_file_path| {
                create_dir_all(data_file_path.parent().unwrap())?;
                Ok(data_file_path)
            })
            .and_then(|data_file_path| Ok(File::create(data_file_path)?))
            .and_then(|fp| {
                let buf_writer = BufWriter::with_capacity(128, fp);
                serde_json::to_writer(buf_writer, &self.state)?;
                Ok(())
            })
    }

    pub(super) fn start_releaser_worker(
        &self,
        tx: mpsc::Sender<Result<bool, Error>>,
    ) -> Result<(), Error> {
        use std::thread;

        let mut releaser = (*self.releaser.borrow()).clone();
        let current_version = self.current_version().clone();
        thread::Builder::new().spawn(move || {
            let outcome: Result<(), Error> = env::workflow_data()
                .ok_or_else(|| err_msg("missing env variable for data dir"))
                .and_then(|mut dir| {
                    dir.push(LATEST_UPDATE_INFO_CACHE_FN_ASYNC);
                    Ok(dir)
                })
                .and_then(|p| {
                    let update_avail = releaser
                        .latest_version()
                        .and_then(|v| Ok((current_version < v, v)))
                        .and_then(|(r, v)| {
                            Self::write_last_check_status(&p, if r { Some(v) } else { None })?;
                            Ok(r)
                        });
                    Ok(tx.send(update_avail)?)
                });
            if let Err(error) = outcome {
                eprint!("worker outcome is error: {:?}", error);
                tx.send(Err(error))
                    .expect("couldnot send error from thread");
            }
        })?;
        Ok(())
    }
    // write version of latest avail. release (if any) to a cache file
    pub(super) fn write_last_check_status(
        p: &PathBuf,
        version: Option<Version>,
    ) -> Result<(), Error> {
        File::create(p).and_then(|fp| {
            let buf_writer = BufWriter::with_capacity(128, fp);
            serde_json::to_writer(buf_writer, &version)?;
            Ok(())
        })?;
        Ok(())
    }

    // read version of latest avail. release (if any) from a cache file
    pub(super) fn read_last_check_status(p: &PathBuf) -> Result<Option<Version>, Error> {
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
