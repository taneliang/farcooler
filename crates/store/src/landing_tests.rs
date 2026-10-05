use super::*;

fn main_workspace() -> (Store, Workspace) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap();
    (store, main)
}

#[test]
fn a_workspace_that_said_nothing_has_chosen_nothing() {
    let (store, main) = main_workspace();
    let landing = store.get_landing(main.id).unwrap();
    assert_eq!(landing, Landing::default());
    assert_eq!(landing.mode, None, "not chosen is not direct");
    assert!(!landing.pr_cost_line, "cost is off until the owner opts in");
}

#[test]
fn each_field_given_is_set_and_each_absent_one_is_left() {
    let (store, main) = main_workspace();
    let patch = LandingPatch { mode: Some(LandingMode::PullRequests), base: Some("trunk".into()), ..Default::default() };
    let w = store.set_landing(main.id, main.resource_version, &patch).unwrap();
    assert_eq!(w.resource_version, main.resource_version + 1, "a setting moves the version, as the others do");
    let after_first = store.get_landing(main.id).unwrap();
    assert_eq!(after_first.mode, Some(LandingMode::PullRequests));
    assert_eq!(after_first.base.as_deref(), Some("trunk"));

    let patch = LandingPatch { pr_cost_line: Some(true), budget_lines: Some(400), ..Default::default() };
    store.set_landing(main.id, w.resource_version, &patch).unwrap();
    let after_second = store.get_landing(main.id).unwrap();
    assert_eq!(after_second.mode, Some(LandingMode::PullRequests), "the mode was not named, so it stays");
    assert_eq!(after_second.base.as_deref(), Some("trunk"));
    assert_eq!(after_second.budget_lines, Some(400));
    assert!(after_second.pr_cost_line);
}

#[test]
fn an_empty_base_and_zero_lines_take_the_value_away() {
    let (store, main) = main_workspace();
    let set = LandingPatch { base: Some("trunk".into()), budget_lines: Some(400), ..Default::default() };
    let w = store.set_landing(main.id, main.resource_version, &set).unwrap();
    let clear = LandingPatch { base: Some("  ".into()), budget_lines: Some(0), ..Default::default() };
    store.set_landing(main.id, w.resource_version, &clear).unwrap();
    let landing = store.get_landing(main.id).unwrap();
    assert_eq!((landing.base, landing.budget_lines), (None, None));
}

#[test]
fn a_stale_version_conflicts_and_writes_nothing() {
    let (store, main) = main_workspace();
    let patch = LandingPatch { mode: Some(LandingMode::Direct), ..Default::default() };
    store.set_landing(main.id, main.resource_version, &patch).unwrap();
    let other = LandingPatch { mode: Some(LandingMode::PullRequests), ..Default::default() };
    let err = store.set_landing(main.id, main.resource_version, &other).unwrap_err();
    assert!(matches!(err, DomainError::ResourceConflict), "{err:?}");
    assert_eq!(store.get_landing(main.id).unwrap().mode, Some(LandingMode::Direct));
    let gone = store.set_landing(Uuid::from_u128(9), 1, &other).unwrap_err();
    assert!(matches!(gone, DomainError::NotFound), "{gone:?}");
}

#[test]
fn a_base_that_cannot_be_a_branch_is_refused_before_anything_is_written() {
    let (store, main) = main_workspace();
    for bad in ["has space", "a..b", "-rf", &"x".repeat(256)] {
        let patch = LandingPatch { base: Some(bad.into()), mode: Some(LandingMode::Direct), ..Default::default() };
        match store.set_landing(main.id, main.resource_version, &patch) {
            Err(DomainError::InvalidArgument { what: "base" }) => {}
            other => panic!("{bad:?}: {other:?}"),
        }
    }
    assert_eq!(store.get_landing(main.id).unwrap(), Landing::default());
    assert_eq!(store.get_workspace(main.id).unwrap().resource_version, main.resource_version);
}

#[test]
fn the_row_goes_with_its_workspace() {
    let (store, main) = main_workspace();
    let repo = main.repository_id;
    let second = store.create_workspace(repo, "Second", "sec").unwrap();
    let patch = LandingPatch { mode: Some(LandingMode::Direct), ..Default::default() };
    store.set_landing(second.id, second.resource_version, &patch).unwrap();
    store.delete_workspace(second.id).unwrap();
    let rows: i64 =
        store.conn().query_row("SELECT count(*) FROM workspace_landing", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn the_mode_words_round_trip() {
    for mode in [LandingMode::Direct, LandingMode::PullRequests] {
        assert_eq!(LandingMode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(LandingMode::parse("squash"), None);
}
