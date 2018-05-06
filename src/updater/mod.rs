//! Helper for enabling Alfred workflows to upgrade themselves periodically (Alfred 3)
//!
//! Enable this feature by adding it to your `Cargo.toml`:
//!
//! ```toml
//! alfred = { version = "4", features = ["updater"] }
//! ```
//! Using this module, the workflow author can make Alfred check for latest releases from a remote
//! server within adjustable intervals ([`update_ready_async()`] or [`update_ready()`])
//! (default is 24 hrs).
//!
//! Additionally they can ask Alfred to download the new release to its cache folder for further
//! action [`download_latest()`].
//!
//! For convenience, an associated method [`Updater::gh()`] is available to check
//! for workflows hosted on `github.com`.
//!
//! However, it's possible to check with other servers as long as the [`Releaser`] trait is
//! implemented for the desired remote service.
//! See [`Updater::new()`] documentation if you are hosting your workflow
//! on non a `github.com` service.
//!
//! The `github.com` hosted repository should have release items following `github`'s process.
//! This can be done by tagging a commit and then manually building a release where you
//! attach/upload `YourWorkflow.alfredworkflow` to the release page.
//!
//! The tag should follow all the [semantic versioning] rules.
//! The only exception to those rules is that you can prepend your
//! semantic version tag with ASCII letter `v`: `v0.3.1` or `0.3.1`
//!
//! You can easily create `YourWorkflow.alfredworkflow` file by using the [export feature] of
//! Alfred in its preferences window.
//!
//! ## Note to workflow authors
//! - Depending on network quality, checking if an update is available may take a long time.
//! This module can spawn a worker thread so the check does not block the main flow of your plugin.
//! However given the limitations of Alfred's plugin architecture, the worker thread cannot outlive
//! your plugin's executable. This means that you either have to wait/block for the worker thread,
//! or if it is taking longer than a desirable time, you will have to abandon it.
//! See the example for more details.
//! - Workflow authors should make sure that _released_ workflow bundles have
//! their version set in [Alfred's preferences window]. However, this module provides
//! [`set_version()`] to set the version during runtime.
//!
//! [`Releaser`]: trait.Releaser.html
//! [`Updater`]: struct.Updater.html
//! [`update_ready()`]: struct.Updater.html#method.update_ready
//! [`update_ready_async()`]: struct.Updater.html#method.update_ready_async
//! [`download_latest()`]: struct.Updater.html#method.download_latest
//! [`Updater::gh()`]: struct.Updater.html#method.gh
//! [`Updater::new()`]: struct.Updater.html#method.new
//! [semantic versioning]: https://semver.org
//! [export feature]: https://www.alfredapp.com/help/workflows/advanced/sharing-workflows/
//! [Alfred's preferences window]: https://www.alfredapp.com/help/workflows/advanced/variables/
//! [`set_version()`]: struct.Updater.html#method.set_version
//! [`set_interval()`]: struct.Updater.html#method.set_interval
//!
//! # Example
//!
//! Create an updater for a workflow hosted on `github.com/kballard/alfred-rs`.
//! By default, it will check for new releases every 24 hours.
//! To change the interval, use [`set_interval()`] method.
//!
//! ```rust,no_run
//! # extern crate alfred;
//! # extern crate failure;
//! use alfred::{Item, ItemBuilder, Updater, json};
//! use std::thread;
//! use std::time::Duration;
//!
//! # use std::io;
//! # use failure::Error;
//! # fn produce_items_for_user_to_see<'a>() -> Vec<Item<'a>> {
//! #     Vec::new()
//! # }
//! # fn do_some_other_stuff() {}
//! fn run<'a>() -> Result<Vec<Item<'a>>, Error> {
//!     let updater = Updater::gh("kballard/alfred-rs").expect("cannot initiate Updater");
//!
//!     // If it has been more than UPDATE_INTERVAL since we last checked,
//!     // the method will spawn a worker thread to check for updates.
//!     // Otherwise it will use a cache and immediately send results to `rx`
//!     let rx = updater
//!         .update_ready_async()
//!         .expect("Error in building & spawning worker");
//!
//!     // We'll do some other work that's related to our workflow while waiting:
//!     do_some_other_stuff();
//!     let mut items: Vec<Item> = produce_items_for_user_to_see();
//!
//!     let mut communication;
//!     // 1: We can check if worker thread is done without blocking
//!     {
//!         communication = rx.try_recv();
//!     }
//!     // Or:
//!     // 2: Optionally wait for 1 second and then check without blocking
//!     {
//!         let one_second = Duration::from_millis(1000);
//!         thread::sleep(one_second);
//!         communication = rx.try_recv();
//!     }
//!
//!     match communication {
//!         Ok(msg) => {
//!             // Communication with worker thread was successful
//!             if let Ok(is_ready) = msg {
//!                 // Worker thread successfully fetched us release info. from github
//!                 if is_ready {
//!                     let update_item = ItemBuilder::new(
//!                         "New version is available!"
//!                     ).into_item();
//!                     items.push(update_item);
//!                 }
//!             }
//!         }
//!         Err(_) => { /* worker thread wasn't successful */ }
//!     }
//!     Ok(items)
//! }
//!
//! fn main() {
//!     if let Ok(ref items) = run() {
//!         json::write_items(io::stdout(), items);
//!     }
//! }
//! ```
//!
//! An *issue* with above example can be when user is on a poor network or server is unresponsive.
//! In this case, the above snippet will try to call server every time workflow is invoked
//! by Alfred until the operation succeeds.

use chrono::prelude::*;
use env;
use failure::{err_msg, Error};
use reqwest;
use semver::Version;
use serde_json;
use std::cell::Cell;
use std::cell::RefCell;
use std::env as StdEnv;
use std::fs::{create_dir_all, remove_file, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use time::Duration;
use url::Url;
use url_serde;

mod imp;
mod releaser;

#[cfg(test)]
mod tests;

pub use self::releaser::GithubReleaser;
pub use self::releaser::Releaser;

use self::imp::default_interval;

// TODO: Update Releaser trait so we don't need two methods (lastest_version and downloadable_url)
//     Only one method (latest_release?) should return both version and a download url.

/// Struct to check for & download the latest release of workflow from a remote server.
pub struct Updater<T>
where
    T: Releaser,
{
    state: UpdaterState,
    releaser: RefCell<T>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdaterState {
    current_version: Version,
    avail_version: RefCell<Option<UpdateInfo>>,
    last_check: Cell<Option<DateTime<Utc>>>,
    #[serde(skip)]
    worker_state: RefCell<Option<MPSCState>>,
    #[serde(skip, default = "default_interval")]
    update_interval: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct UpdateInfo {
    avail_version: Version,
    #[serde(with = "url_serde")]
    downloadable_url: Url,
}

#[derive(Debug)]
struct MPSCState {
    recvd_payload: RefCell<Option<Result<Option<UpdateInfo>, Error>>>,
    rx: RefCell<Option<Receiver<Result<Option<UpdateInfo>, Error>>>>,
}

impl Updater<GithubReleaser> {
    /// Create an `Updater` object that will interface with a `github` repository.
    ///
    /// The `repo_name` should be in `user_name/repository_name` form. See the
    /// [module level documentation](./index.html) for full example and description.
    ///
    /// ```rust
    /// # extern crate alfred;
    /// use alfred::Updater;
    /// # use std::env;
    /// # fn main() {
    /// # env::set_var("alfred_workflow_uid", "abcdef");
    /// # env::set_var("alfred_workflow_data", env::temp_dir());
    /// # env::set_var("alfred_workflow_version", "0.0.0");
    /// let updater = Updater::gh("kballard/alfred-rs").expect("cannot initiate Updater");
    /// # }
    /// ```
    ///
    /// This only creates an `Updater` without performing any network operations.
    /// To check availability of a new release use [`update_ready()`] method.
    ///
    /// To download an available release use [`download_latest()`] method.
    ///
    /// # Errors
    /// Error will happen during calling this method if:
    /// - `Updater` state cannot be read/written during instantiation, or
    /// - The workflow version cannot be parsed as semantic version compatible identifier.
    ///
    /// [`update_ready()`]: struct.Updater.html#method.update_ready
    /// [`download_latest()`]: struct.Updater.html#method.download_latest
    pub fn gh<S>(repo_name: S) -> Result<Self, Error>
    where
        S: Into<String>,
    {
        let releaser = GithubReleaser::new(repo_name);

        Self::load_or_new(releaser)
    }
}

impl<T> Updater<T>
where
    T: Releaser + Send + 'static,
{
    /// Create an `Updater` object that will interface with a remote repository for updating operations.
    ///
    /// How the `Updater` interacts with the remote server should be implemented using the [`Releaser`]
    /// trait.
    ///
    /// ```rust
    /// # extern crate alfred;
    /// # extern crate semver;
    /// # extern crate failure;
    /// # extern crate url;
    /// use std::io;
    ///
    /// use semver::Version;
    /// use alfred::Updater;
    /// use alfred::updater::Releaser;
    /// # use std::env;
    /// # use failure::Error;
    /// # use url::Url;
    /// # fn main() {
    /// # env::set_var("alfred_workflow_uid", "abcdef");
    /// # env::set_var("alfred_workflow_data", env::temp_dir());
    /// # env::set_var("alfred_workflow_version", "0.0.0");
    /// # env::set_var("alfred_workflow_name", "NameName");
    ///
    /// #[derive(Clone)]
    /// struct RemoteCIReleaser {/* inner */};
    ///
    /// // You need to actually implement the trait, following is just a mock.
    /// impl Releaser for RemoteCIReleaser {
    ///     fn new<S: Into<String>>(project_id: S) -> Self {
    ///         RemoteCIReleaser {}
    ///     }
    ///     fn downloadable_url(&self) -> Result<Url, Error> {
    ///         Ok(Url::parse("https://ci.remote.cc")?)
    ///     }
    ///     fn latest_version(&mut self) -> Result<Version, Error> {
    ///         Ok(Version::from((1, 0, 12)))
    ///     }
    /// }
    ///
    /// let updater: Updater<RemoteCIReleaser> =
    ///     Updater::new("my_hidden_proj").expect("cannot initiate Updater");
    /// # }
    /// ```
    ///
    /// Note that the method only creates an `Updater` without performing any network operations.
    ///
    /// To check availability of a new release use [`update_ready()`] method.
    ///
    /// To download an available release use [`download_latest()`] method.
    ///
    /// # Errors
    /// Error will happen during calling this method if:
    /// - `Updater` state cannot be read/written during instantiation, or
    /// - The workflow version cannot be parsed as a semantic version compatible identifier.
    ///
    /// [`update_ready()`]: struct.Updater.html#method.update_ready
    /// [`download_latest()`]: struct.Updater.html#method.download_latest
    /// [`Releaser`]: trait.Releaser.html
    pub fn new<S>(repo_name: S) -> Result<Updater<T>, Error>
    where
        S: Into<String>,
    {
        let releaser = Releaser::new(repo_name);
        Self::load_or_new(releaser)
    }

    /// Set workflow's version to `version`.
    ///
    /// Content of `version` needs to follow semantic versioning.
    ///
    /// This method is provided so workflow authors can set the version from within the Rust code.
    ///
    /// For example, by reading cargo or git info during compile time and using this method to
    /// assign the version to workflow.
    ///
    /// # Example
    ///
    /// ```rust
    /// # extern crate alfred;
    /// # extern crate failure;
    /// # use alfred::Updater;
    /// # use std::env;
    /// # use failure::Error;
    /// # fn ex_set_version() -> Result<(), Error> {
    /// # env::set_var("alfred_workflow_uid", "abcdef");
    /// # env::set_var("alfred_workflow_data", env::temp_dir());
    /// # env::set_var("alfred_workflow_version", "0.0.0");
    /// let mut updater = Updater::gh("kballard/alfred-rs")?;
    /// updater.set_version("0.23.3");
    /// # Ok(())
    /// # }
    ///
    /// # fn main() {
    /// #     ex_set_version();
    /// # }
    /// ```
    /// An alternative (recommended) way of setting version is through [Alfred's preferences window].
    ///
    /// [Alfred's preferences window]: https://www.alfredapp.com/help/workflows/advanced/variables/
    ///
    /// # Panics
    /// The method will panic if the passed value `version` cannot be parsed as a semantic version compatible string.
    pub fn set_version<S: AsRef<str>>(&mut self, version: S) {
        self.state.current_version = Version::parse(version.as_ref())
            .expect("version should follow semantic version rules.");
        StdEnv::set_var("alfred_workflow_version", version.as_ref());
    }

    /// Set the interval between checks for a newer release (in seconds)
    ///
    /// # Example
    /// Set interval to be 7 days
    ///
    /// ```rust
    /// # extern crate alfred;
    /// # use alfred::Updater;
    /// # use std::env;
    /// # fn main() {
    /// # env::set_var("alfred_workflow_uid", "abcdef");
    /// # env::set_var("alfred_workflow_data", env::temp_dir());
    /// # env::set_var("alfred_workflow_version", "0.0.0");
    /// let mut updater =
    ///     Updater::gh("kballard/alfred-rs").expect("cannot initiate Updater");
    /// updater.set_interval(7 * 24 * 60 * 60);
    /// # }
    /// ```
    pub fn set_interval(&mut self, tick: i64) {
        self.set_update_interval(tick);
    }

    /// Checks if a new update is available (non-blocking).
    ///
    /// This method will fetch the latest release information from repository
    /// and compare it to the current release of the workflow. The repository should
    /// tag each release according to semantic version scheme for this to work.
    ///
    /// The spawned thread will attempt to make a network call to fetch metadata of releases
    /// *only if* UPDATE_INTERVAL seconds has passed since the last network call.
    ///
    /// All calls, which happen before the UPDATE_INTERVAL seconds, will use a local cache
    /// to report availability of a release.
    ///
    /// For `Updater`s talking to `github.com`, this method will only fetch a small metadata information
    /// to extract the version of the latest release.
    ///
    /// ## Note
    /// Unlike [`update_ready()`], this method does not block the current thread. In order to use the
    /// possible result produced by the spawned thread, you need to call either [`recv()`] (blocking) or
    /// [`try_recv()`] (non-blocking) method of the returned `Receiver` before your application's
    /// `main()` exits.
    ///
    /// Note that the worker thread may not have been done by the time you call [`try_recv()`], which
    /// results a no-response error and the main thread continuing its normal execution without blocking.
    ///
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # extern crate alfred;
    /// # extern crate failure;
    /// # use failure::Error;
    /// # use alfred::Updater;
    /// # use std::env;
    /// # fn do_some_other_stuff() {}
    /// # fn test_async() -> Result<(), Error> {
    /// let mut updater = Updater::gh("kballard/alfred-rs")?;
    ///
    /// let rx = updater.update_ready_async().expect("Error in building & spawning worker");
    ///
    /// // We'll do some other work that's related to our workflow while waiting
    /// do_some_other_stuff();
    ///
    /// // We can now check if update is ready using two methods on `rx`:
    /// // 1- Block and wait until it receives results or errors
    /// let communication = rx.recv();
    ///
    /// // 2- Or without blocking, check if the worker thread sent the results.
    /// //    If the worker thread is still busy, the `communication` will be an `Err`
    /// let communication = rx.try_recv();
    ///
    /// if let Ok(msg) = communication { // Communication with worker thread was successful
    ///
    ///     if let Ok(ready) = msg {
    ///         // No error happened during operation of worker thread and a `msg` containing
    ///         // the `ready` flag is available.
    ///         // Use it to see if an update is available or not.
    ///     } else {
    ///         /* worker thread wasn't successful */
    ///     }
    ///
    /// }
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// # test_async();
    /// # }
    /// ```
    /// [`Releaser`]: trait.Releaser.html
    /// [`Receiver`]: https://doc.rust-lang.org/std/sync/mpsc/struct.Receiver.html
    /// [`Result`]: https://doc.rust-lang.org/std/result/enum.Result.html
    /// [`update_ready()`]: struct.Updater.html#method.update_ready
    /// [`recv()`]: https://doc.rust-lang.org/std/sync/mpsc/struct.Receiver.html#method.recv
    /// [`try_recv()`]: https://doc.rust-lang.org/std/sync/mpsc/struct.Receiver.html#method.try_recv
    pub fn init(&self) -> Result<(), Error> {
        use self::imp::LATEST_UPDATE_INFO_CACHE_FN_ASYNC;
        use std::sync::mpsc;

        // file for status of last update check
        let p = Self::build_data_fn()?.with_file_name(LATEST_UPDATE_INFO_CACHE_FN_ASYNC);

        let (tx, rx) = mpsc::channel();

        if self.last_check().is_none() {
            self.set_last_check(Utc::now());
            self.save()?;
            // This send is always successful
            tx.send(Ok(None)).unwrap();
        } else if self.due_to_check() {
            // it's time to talk to remote server
            self.start_releaser_worker(tx, p)?;
        } else {
            // let last_check = Self::read_last_check_status(&p).unwrap_or(None);
            let status = Self::read_last_check_status(&p)
                .map(|last_check| {
                    last_check.and_then(|info| {
                        if self.current_version() < &info.avail_version {
                            Some(info)
                        } else {
                            None
                        }
                    })
                })
                .or(Ok(None));
            tx.send(status).unwrap();
        }
        *self.state.worker_state.borrow_mut() = Some(MPSCState {
            recvd_payload: RefCell::new(None),
            rx: RefCell::new(Some(rx)),
        });
        Ok(())
    }

    pub fn update_ready(&self) -> Result<bool, Error> {
        if self.state.worker_state.borrow().is_none() {
            self.update_ready_sync()
        } else {
            self.update_ready_async_()
        }
    }

    /// Check if it is time to ask remote server for latest updates.
    ///
    /// It returns `true` if it has been more than UPDATE_INTERVAL seconds since we last
    /// checked with server (i.e. ran [`update_ready()`]), otherwise returns false.
    ///
    /// [`update_ready()`]: struct.Updater.html#method.update_ready
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # extern crate alfred;
    /// # extern crate failure;
    /// # use alfred::Updater;
    /// # use failure::Error;
    /// # fn run() -> Result<(), Error> {
    /// let mut updater = Updater::gh("kballard/alfred-rs")?;
    ///
    /// // Assuming it is has been UPDATE_INTERVAL seconds since last time we ran the
    /// // `update_ready()` and there actually exists a new release:
    /// assert_eq!(true, updater.due_to_check());
    /// # Ok(())
    /// # }
    /// # fn main() {
    /// # run();
    /// # }
    /// ```
    ///
    pub fn due_to_check(&self) -> bool {
        self.last_check().map_or(true, |dt| {
            Utc::now().signed_duration_since(dt) > Duration::seconds(self.update_interval())
        })
    }

    /// Function to download and save the latest release into workflow's cache dir.
    ///
    /// If the download and save operations are both successful, it returns name of file in which the
    /// downloaded Alfred workflow bundle is saved.
    ///
    /// The downloaded workflow will be saved in dedicated cache folder of the workflow, and it
    /// will be always renamed to `latest_release_WORKFLOW-UID.alfredworkflow`
    ///
    /// To install the downloaded release, your workflow needs to somehow open the saved file.
    ///
    /// Within shell, it can be installed by issuing something like:
    /// ```bash
    /// open -b com.runningwithcrayons.Alfred-3 latest_release_WORKFLOW-UID.alfredworkflow
    /// ```
    ///
    /// Or you can add "Run script" object to your workflow and use environment variables set by
    /// Alfred to automatically open the downloaded release:
    /// ```bash
    /// open -b com.runningwithcrayons.Alfred-3 "$alfred_workflow_cache/latest_release_$alfred_workflow_uid.alfredworkflow"
    /// ```
    ///
    /// # Note
    ///
    /// The method may take longer than other Alfred-based actions to complete. Workflow authors using this crate
    /// should implement strategies to prevent unpleasant long blocks of user's typical work flow.
    ///
    /// One option to initiate the download and upgrade process is to invoke your executable with a
    /// different argument. The following snippet can be tied to a dedicated Alfred **Hotkey**
    /// or **Script Filter** so that it is only executed when user explicitly asks for it:
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # extern crate alfred;
    /// # extern crate failure;
    /// # use alfred::Updater;
    /// # use std::io;
    /// use alfred::{ItemBuilder, json};
    ///
    /// # fn main() {
    /// # let updater =
    /// #    Updater::gh("kballard/alfred-rs").expect("cannot initiate Updater");
    /// # let cmd_line_download_flag = true;
    /// if cmd_line_download_flag && updater.update_ready().unwrap() {
    ///     match updater.download_latest() {
    ///         Ok(downloaded_fn) => {
    ///           json::write_items(io::stdout(), &[
    ///               ItemBuilder::new("New version of workflow is available!")
    ///                            .subtitle("Click to upgrade!")
    ///                            .arg(downloaded_fn.to_str().unwrap())
    ///                            .variable("update_ready", "yes")
    ///                            .valid(true)
    ///                            .into_item()
    ///           ]);
    ///         },
    ///         Err(e) => {
    ///             // Show an error message to user or log it.
    ///         }
    ///     }
    /// }
    /// #    else {
    /// #    }
    /// # }
    /// ```
    ///
    /// For the above example to automatically work, you then need to connect the output of the script
    /// to an **Open File** action so that Alfred can install/upgrade the new version.
    ///
    /// As suggested in above example, you can add an Alfred variable to the item so that your workflow
    /// can use it for further processing.
    ///
    /// # Errors
    /// Downloading latest workflow can fail if network error, file error or Alfred environment variable
    /// errors happen, or if `Releaser` cannot produce a usable download url.
    pub fn download_latest(&self) -> Result<PathBuf, Error> {
        let url = self.releaser.borrow().downloadable_url()?;
        let client = reqwest::Client::new();

        client
            .get(url)
            .send()?
            .error_for_status()
            .map_err(|e| e.into())
            .and_then(|mut resp| {
                // Get workflow's dedicated cache folder & build a filename
                let latest_release_downloaded_fn = env::workflow_cache()
                    .ok_or_else(|| err_msg("missing env variable for cache dir"))
                    .and_then(|mut cache_dir| {
                        env::workflow_uid()
                            .ok_or_else(|| err_msg("missing env variable for uid"))
                            .and_then(|ref uid| {
                                cache_dir
                                    .push(["latest_release_", uid, ".alfredworkflow"].concat());
                                Ok(cache_dir)
                            })
                    })?;
                // Save the file
                File::create(&latest_release_downloaded_fn)
                    .map_err(|e| e.into())
                    .and_then(|fp| {
                        let mut buf_writer = BufWriter::with_capacity(0x10_0000, fp);
                        resp.copy_to(&mut buf_writer)?;
                        Ok(())
                    })
                    .or_else(|e: Error| {
                        let _ = remove_file(&latest_release_downloaded_fn);
                        Err(e)
                    })?;
                Ok(latest_release_downloaded_fn)
            })
    }
}
