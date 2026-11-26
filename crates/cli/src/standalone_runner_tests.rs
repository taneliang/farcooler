use super::*;

#[tokio::test]
async fn standalone_update_refuses_a_landing_and_wait_does_not_admit_another() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let database = home.join("farcooler.db");
    let socket = PathBuf::from(format!("/private/tmp/farcoolerd-{}.sock", uuid::Uuid::now_v7()));
    let landing = File::options().write(true).create(true).truncate(false)
        .open(home.join("landing-update.lock")).unwrap();
    landing.lock_shared().unwrap();

    let result = reserve(home, &socket, &database, false).await;
    assert!(result.err().unwrap().to_string().contains("--wait"));
    let mut waiting = Box::pin(reserve(home, &socket, &database, true));
    tokio::select! {
        _ = &mut waiting => panic!("the updater did not wait for the landing"),
        _ = tokio::time::sleep(Duration::from_millis(30)) => {}
    }
    let admission = File::open(home.join("landing-update-waiters.lock")).unwrap();
    assert!(admission.try_lock().is_err(), "another landing must wait behind the update");

    drop(landing);
    let reserved = tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
    drain(reserved.link, &socket, &database, reserved.legacy).await.unwrap();
    assert!(landing_update::try_update(home).unwrap().is_none());
    drop((reserved._waiting, reserved._landing));
    assert!(landing_update::try_update(home).unwrap().is_some());
}

#[test]
fn standalone_update_switches_all_tools_through_one_pointer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let current = root.join("current");
    let one = Path::new("releases/10-old");
    let two = Path::new("releases/11-new");
    for (release, marker) in [(one, b"old"), (two, b"new")] {
        fs::create_dir_all(root.join(release)).unwrap();
        for binary in [CLI, DAEMON, INSTALLER] {
            fs::write(root.join(release).join(binary), marker).unwrap();
        }
    }
    atomic_link(&current, one).unwrap();
    let bin = root.join("bin");
    for binary in [CLI, DAEMON, INSTALLER] {
        atomic_link(&bin.join(binary), &current.join(binary)).unwrap();
        assert_eq!(fs::read(bin.join(binary)).unwrap(), b"old");
    }
    atomic_link(&current, two).unwrap();
    assert_eq!(release_build(&fs::read_link(&current).unwrap()), Some(11));
    for binary in [CLI, DAEMON, INSTALLER] {
        assert_eq!(fs::read(bin.join(binary)).unwrap(), b"new");
        assert_eq!(fs::read(root.join(one).join(binary)).unwrap(), b"old");
    }
}

#[test]
fn standalone_migration_replaces_only_links_from_the_selected_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Far Cooler Canary.app");
    let current = dir.path().join("current");
    let cli = dir.path().join(CLI);
    atomic_link(&cli, &app.join("Contents/Resources/farcooler")).unwrap();
    assert!(check_link(&cli, &current.join(CLI), Some(&app)).is_ok());
    assert!(check_link(&cli, &current.join(CLI), None).is_err());
    assert!(check_link(&cli, &current.join(CLI), Some(dir.path())).is_err());
    let daemon = dir.path().join(DAEMON);
    fs::write(&daemon, b"an unrelated installation").unwrap();
    assert!(check_link(&daemon, &current.join(DAEMON), Some(&app)).is_err());
}

#[test]
fn standalone_launch_agent_is_valid_xml_and_preserves_child_processes() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("Test & <user>");
    let install = Install { home, root: dir.path().join("runner"), domain: "user/999".into() };
    let path = dir.path().join("job.plist");
    fs::write(&path, install.plist()).unwrap();
    let read = |key| {
        let out = Command::new("/usr/bin/plutil").args(["-extract", key, "raw", "-o", "-"])
            .arg(&path).output().unwrap();
        success(out, key).unwrap().trim().to_owned()
    };
    assert_eq!(read("LimitLoadToSessionType"), "Background");
    assert_eq!(read("RunAtLoad"), "true");
    assert_eq!(read("AbandonProcessGroup"), "true");
    assert_eq!(read("KeepAlive.SuccessfulExit"), "false");
    assert!(read("EnvironmentVariables.PATH").contains("Test & <user>"));
}

#[test]
fn standalone_update_replaces_owned_older_plists_and_can_restore_them() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_owned();
    let install = Install { root: install_root(&home), home, domain: "user/999".into() };
    for (label, domain, path, desired) in [
        (LABEL, install.domain.clone(), install.plist_path(), install.plist()),
        (LOGIN_LABEL, install.gui_domain(), install.login_plist_path(), install.login_plist()),
    ] {
        let older = desired.replace("<key>RunAtLoad</key><true/>", "<key>RunAtLoad</key><false/>");
        assert_ne!(older, desired);
        write_plist(&path, older.as_bytes(), false).unwrap();
        // Binary plists also have a valid ownership record.
        success(Command::new("/usr/bin/plutil").args(["-convert", "binary1"])
            .arg(&path).output().unwrap(), "convert fixture plist").unwrap();
        let old_bytes = fs::read(&path).unwrap();
        let agent = LaunchAgent::inspect(&install, label, domain, path.clone(), desired.clone()).unwrap();
        assert!(agent.changed());
        write_plist(&agent.path, agent.desired.as_bytes(), true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), desired);
        agent.restore().unwrap();
        assert_eq!(fs::read(&path).unwrap(), old_bytes);
    }
}

#[test]
fn standalone_update_refuses_foreign_launch_agents() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_owned();
    let install = Install { root: install_root(&home), home, domain: "user/999".into() };
    let desired = install.plist();
    let path = install.plist_path();
    let daemon = xml(&install.root.join("current").join(DAEMON));
    for foreign in [
        desired.replace(LABEL, "com.example.unrelated"),
        desired.replace(&daemon, "/bin/sleep"),
        desired.replace(&daemon, &format!("{}/../other", xml(&install.root))),
        desired.replace("<key>ProgramArguments</key>", "<key>Program</key><string>/bin/sleep</string><key>ProgramArguments</key>"),
    ] {
        write_plist(&path, foreign.as_bytes(), true).unwrap();
        let result = LaunchAgent::inspect(&install, LABEL, install.domain.clone(), path.clone(), desired.clone());
        assert!(result.err().unwrap().to_string().contains("LaunchAgent conflict"));
        assert_eq!(fs::read_to_string(&path).unwrap(), foreign);
    }

    let desired = install.login_plist();
    let path = install.login_plist_path();
    let foreign = desired.replace(&xml(&install.plist_path()), "/tmp/another.plist");
    write_plist(&path, foreign.as_bytes(), false).unwrap();
    let result = LaunchAgent::inspect(&install, LOGIN_LABEL, install.gui_domain(), path, desired);
    assert!(result.err().unwrap().to_string().contains("LaunchAgent conflict"));
}

#[test]
fn standalone_migration_refuses_a_running_app_for_the_same_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path();
    assert!(!app_owns_runtime(runtime).unwrap());
    let app = File::create(runtime.join("app.sock.lock")).unwrap();
    app.lock().unwrap();
    assert!(app_owns_runtime(runtime).unwrap());
    drop(app);
    assert!(!app_owns_runtime(runtime).unwrap());
}
