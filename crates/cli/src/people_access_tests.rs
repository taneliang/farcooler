use super::*;

fn rows() -> pb::MemberList {
    let device = |id: &str, person: &str, access: &str, cap: &str, check: bool| pb::DeviceAccess {
        client_id: id.into(),
        person: person.into(),
        access: access.into(),
        cap: cap.into(),
        check,
    };
    pb::MemberList {
        members: vec![pb::Member { handle: "bob".into(), name: "Bob".into(), access: "member".into(), ..Default::default() }],
        device_access: vec![
            device("ipad-o", "", "owner", "viewer", false),
            device("legacy-t", "", "owner", "member", true),
            device("phone-b", "bob", "member", "", false),
        ],
        ..Default::default()
    }
}

#[test]
fn each_device_says_whose_it_is_and_what_holds_it_down() {
    let text = access_lines(&rows());
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("owner") && lines[0].contains("ipad-o") && lines[0].ends_with("held at viewer"), "{}", lines[0]);
    assert!(lines[1].contains("check: added by a member's device"), "{}", lines[1]);
    assert!(lines[2].starts_with("bob") && lines[2].trim_end().ends_with("member"), "{}", lines[2]);
}

#[test]
fn json_keeps_the_owner_apart_from_the_members() {
    let v = access_json(&rows());
    assert_eq!(v["people"][0], json!({ "handle": null, "name": "owner", "access": "owner" }));
    assert_eq!(v["people"][1]["access"], "member");
    assert_eq!(v["devices"][0], json!({ "client_id": "ipad-o", "person": null, "access": "owner", "cap": "viewer", "check": false }));
    assert_eq!(v["devices"][2]["cap"], Value::Null);
    assert_eq!(v["devices"][1]["check"], true);
}

#[test]
fn a_runner_that_has_recorded_nothing_says_so() {
    assert_eq!(access_lines(&pb::MemberList::default()), NO_ACCESS);
}
