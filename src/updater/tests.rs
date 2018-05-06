use self::releaser::tests::setup_mock_server;
#[cfg(not(feature = "ci"))]
use self::releaser::GithubReleaser;
use self::releaser::MOCK_RELEASER_REPO_NAME;
use super::*;
use std::ffi::OsStr;
use tempfile::Builder;
const VERSION_TEST: &str = "0.10.5";
const VERSION_TEST_NEW: &str = "0.11.1"; // should match what the mock server replies for new version.

#[test]
fn it_tests_settings_filename() {
    setup_workflow_env_vars(true);
    let updater_state_fn = Updater::<GithubReleaser>::build_data_fn().unwrap();
    assert_eq!(
        "workflow.B0AC54EC-601C-YouForgotTo___Name_Your_Own_Work_flow_-updater.json",
        updater_state_fn.file_name().unwrap().to_str().unwrap()
    );
}

#[test]
fn it_ignores_saved_version_after_an_upgrade() {
    // Make sure a freshly upgraded workflow does not use version info from saved state
    setup_workflow_env_vars(true);
    let _m = setup_mock_server(200);

    {
        let updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        assert_eq!(VERSION_TEST, format!("{}", updater.current_version()));
        // First update_ready is always false.
        assert_eq!(
            false,
            updater.update_ready().expect("couldn't check for update")
        );
    }

    {
        // Next check it reports a new version since mock server has a release for us
        let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        updater.set_interval(0);
        assert_eq!(
            true,
            updater.update_ready().expect("couldn't check for update")
        );
        assert_eq!(VERSION_TEST, format!("{}", updater.current_version()));
    }

    // Mimic the upgrade process by bumping the version
    StdEnv::set_var("alfred_workflow_version", VERSION_TEST_NEW);
    {
        let updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        // Updater should pick up new version rather than using saved one
        assert_eq!(VERSION_TEST_NEW, format!("{}", updater.current_version()));
        // No more updates
        assert_eq!(
            false,
            updater.update_ready().expect("couldn't check for update")
        );
    }
}

#[test]
fn it_ignores_saved_version_after_an_upgrade_async() {
    // Make sure a freshly upgraded workflow does not use version info from saved state
    setup_workflow_env_vars(true);
    let _m = setup_mock_server(200);

    {
        let updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        assert_eq!(VERSION_TEST, format!("{}", updater.current_version()));
        // First update_ready is always false.
        assert_eq!(
            false,
            updater.update_ready().expect("couldn't check for update")
        );
    }

    {
        // Next check it reports a new version since mock server has a release for us
        let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        updater.set_interval(0);
        updater.init().expect("couldn't init worker");

        assert_eq!(
            true,
            updater.update_ready().expect("couldn't check for update")
        );
        assert_eq!(VERSION_TEST, format!("{}", updater.current_version()));
    }

    // Mimic the upgrade process by bumping the version
    StdEnv::set_var("alfred_workflow_version", VERSION_TEST_NEW);
    {
        let updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        // Updater should pick up new version rather than using saved one
        assert_eq!(VERSION_TEST_NEW, format!("{}", updater.current_version()));
        updater.init().expect("couldn't init worker");
        // No more updates
        assert_eq!(
            false,
            updater.update_ready().expect("couldn't check for update")
        );
    }
}

#[test]
#[should_panic(expected = "ClientError(BadRequest)")]
fn it_handles_server_error() {
    setup_workflow_env_vars(true);
    let _m = setup_mock_server(200);

    let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
    // First update_ready is always false.
    assert_eq!(
        false,
        updater.update_ready().expect("couldn't check for update")
    );

    // Next check will be immediate
    updater.set_interval(0);
    let _m = setup_mock_server(400);
    // This should panic with a BadRequest (400) error.
    updater.update_ready().unwrap();
}

#[test]
#[should_panic(expected = "ClientError(BadRequest)")]
fn it_handles_server_error_async() {
    setup_workflow_env_vars(true);

    {
        let _m = setup_mock_server(200);
        let updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        assert_eq!(VERSION_TEST, format!("{}", updater.current_version()));
        updater.init().expect("couldn't init worker");
        // First update_ready is always false.
        assert_eq!(
            false,
            updater.update_ready().expect("couldn't check for update")
        );
    }

    {
        let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        // Next check will be immediate
        updater.set_interval(0);
        updater.init().expect("couldn't init worker");
        let _m = setup_mock_server(400);
        // This should panic with a BadRequest (400) error.
        updater.update_ready().unwrap();
    }
}

#[test]
fn it_caches_workers_payload() {
    setup_workflow_env_vars(true);

    {
        let _m = setup_mock_server(200);
        let updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        assert_eq!(VERSION_TEST, format!("{}", updater.current_version()));
        updater.init().expect("couldn't init worker");
        // First update_ready is always false.
        assert_eq!(
            false,
            updater.update_ready().expect("couldn't check for update")
        );
    }
    {
        let _m = setup_mock_server(200);
        let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        // Next check will be immediate
        updater.set_interval(0);
        updater.init().expect("couldn't init worker");
        assert_eq!(
            true,
            updater.update_ready().expect("couldn't check for update"),
        );

        // Consequenst calls to update_ready should cache the payload.
        let _m = setup_mock_server(400);
        assert_eq!(
            true,
            updater.update_ready().expect("couldn't check for update"),
        );
        assert_eq!(
            true,
            updater.update_ready().expect("couldn't check for update"),
        );
        assert_eq!(
            true,
            updater.update_ready().expect("couldn't check for update"),
        );
    }
}

#[test]
fn it_get_latest_info_from_releaser() {
    setup_workflow_env_vars(true);
    let _m = setup_mock_server(200);

    {
        // Blocking
        let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        assert_eq!(
            false,
            updater.update_ready().expect("couldn't check for update")
        );

        // Next check will be immediate
        updater.set_interval(0);

        assert!(updater.update_ready().expect("couldn't check for update"));
    }
    {
        // Non-blocking
        let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
        // Next check will be immediate
        updater.set_interval(0);

        // Start async worker
        updater.init().expect("couldn't init worker");
        assert!(updater.update_ready().expect("couldn't check for update"));
    }
}

#[test]
// #[cfg(not(feature = "ci"))]
fn it_does_one_network_call_per_interval() {
    setup_workflow_env_vars(true);
    let _m = setup_mock_server(200);

    let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");

    // Calling update_ready on first run of workflow will return false since we assume workflow
    // was just downloaded.
    assert!(!updater.update_ready().expect("couldn't check for update"));

    // Next check will be immediate
    updater.set_interval(0);

    // Next update_ready will make a network call
    assert!(updater.update_ready().expect("couldn't check for update"));

    // Increase interval
    updater.set_interval(86400);
    assert!(!updater.due_to_check());

    // make mock server return error. This way we can test that no network call was made
    // assuming Updater can read its cache file successfully
    let _m = setup_mock_server(503);
    let t = updater.update_ready();
    assert!(t.is_ok());
    // Make sure we stil report update is ready
    assert_eq!(true, t.unwrap());
}

#[test]
fn it_tests_download() {
    setup_workflow_env_vars(true);
    let _m = setup_mock_server(200);

    let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
    assert_eq!(
        false,
        updater.update_ready().expect("couldn't check for update")
    );

    // Next check will be immediate
    updater.set_interval(0);
    // Force current version to be really old.
    updater.set_version("0.0.1");
    assert!(updater.update_ready().expect("couldn't check for update"));
    assert!(updater.download_latest().is_ok());
}

// #[test]
// fn it_tests_async_updates_1() {
//     // This test will wait for the other thread to finish (rx.recv() blocks)
//     setup_workflow_env_vars(true);
//     let _m = setup_mock_server(200);

//     let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");
//     // Next check will be immediate
//     updater.set_interval(0);

//     let rx = updater.update_ready_async().unwrap_or_else(|e| panic!(e));
//     let status = rx.recv();
//     assert!(status.is_ok());
//     assert_eq!(false, status.unwrap().unwrap());
// }

// #[test]
// fn it_tests_async_updates_2() {
//     // This test will only spawn a thread once.
//     // Second call will use a cache since it's not due to check.
//     setup_workflow_env_vars(true);
//     let _m = setup_mock_server(200);

//     let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");

//     {
//         // Calling update_ready on first run of workflow will return false since we assume workflow
//         // was just downloaded.
//         let r = updater
//                 .update_ready_async()
//                 .unwrap_or_else(|e| panic!(e))
//                 .recv()
//                 .unwrap() // Unwrap recv-ing operation result
//                 .unwrap(); // Unwrap contained msg
//         assert_eq!(false, r);
//     }

//     {
//         // Next check will spawn a thread. There should be an update avail. from mock server.
//         updater.set_interval(0);
//         let r = updater
//                 .update_ready_async()
//                 .unwrap_or_else(|e| panic!(e))
//                 .recv()
//                 .unwrap() // Unwrap recv-ing operation result
//                 .unwrap(); // Unwrap contained msg
//         assert_eq!(true, r);
//     }

//     {
//         // make mock server return error. This way we can test that no network call was made
//         // assuming Updater can read its cache file successfully
//         let _m = setup_mock_server(503);
//         // Increase interval
//         updater.set_interval(86400);

//         let t = updater
//             .update_ready_async()
//             .unwrap_or_else(|e| panic!(e))
//             .recv()
//             .unwrap();
//         assert!(t.is_ok()); // No network call was made, otherwise this would've been error
//         assert_eq!(true, t.unwrap());
//     }
// }

// #[test]
// #[should_panic(expected = "ServerError(InternalServerError)")]
// fn it_tests_async_updates_3() {
//     setup_workflow_env_vars(true);
//     let _m = setup_mock_server(200);

//     let mut updater = Updater::gh(MOCK_RELEASER_REPO_NAME).expect("cannot build Updater");

//     {
//         // Calling update_ready on first run of workflow will return false since we assume workflow
//         // was just downloaded.
//         let r = updater
//             .update_ready_async()
//             .unwrap_or_else(|e| panic!(e))
//             .recv()
//             .unwrap()
//             .unwrap();
//         assert_eq!(false, r);
//     }

//     // Next check will spawn a thread.
//     {
//         updater.set_interval(0);
//         // Introduce a server error
//         let _m = setup_mock_server(500);
//         let r = updater.update_ready_async();

//         // Calling and spawning thread should go ok
//         assert!(r.is_ok());

//         // However the received msg should contain error since spawned thread
//         // will get a 500 error from server
//         let msg = r.unwrap().recv().unwrap();
//         assert!(msg.is_err());
//         msg.unwrap();
//     }
// }

pub(super) fn setup_workflow_env_vars(secure_temp_dir: bool) -> PathBuf {
    // Mimic Alfred's environment variables
    let path = if secure_temp_dir {
        Builder::new()
            .prefix("alfred_rs_test")
            .rand_bytes(5)
            .tempdir()
            .unwrap()
            .into_path()
    } else {
        StdEnv::temp_dir()
    };
    {
        let v: &OsStr = path.as_ref();
        StdEnv::set_var("alfred_workflow_data", v);
        StdEnv::set_var("alfred_workflow_cache", v);
        StdEnv::set_var("alfred_workflow_uid", "workflow.B0AC54EC-601C");
        StdEnv::set_var(
            "alfred_workflow_name",
            "YouForgotTo/フ:Name好YouráOwnسWork}flowッ",
        );
        StdEnv::set_var("alfred_workflow_bundleid", "MY_BUNDLE_ID");
        StdEnv::set_var("alfred_workflow_version", VERSION_TEST);
    }
    path
}
