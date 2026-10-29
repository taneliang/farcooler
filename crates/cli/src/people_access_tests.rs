use super::*;

fn rows() -> pb::MemberList {
    let device = |id: &str, person: &str, access: &str, cap: &str, check: bool| pb::DeviceAccess {
        client_id: id.into(),
        person: person.into(),
        access: access.into(),
        cap: cap.into(),
        check,
        ..Default::default()
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
    assert_eq!(
        v["devices"][0],
        json!({ "client_id": "ipad-o", "person": null, "access": "owner", "cap": "viewer", "check": false, "step_up": "", "step_up_code": "" })
    );
    assert_eq!(v["devices"][2]["cap"], Value::Null);
    assert_eq!(v["devices"][1]["check"], true);
}

#[test]
fn a_runner_that_has_recorded_nothing_says_so() {
    assert_eq!(access_lines(&pb::MemberList::default()), NO_ACCESS);
}

// MARK: - The changes the socket makes

struct Link {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
    answer: result::Value,
}

impl DispatchLink for Link {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, farcooler_transport::ClientError> {
        self.sent.push(req);
        Ok(pb::Result { value: Some(self.answer.clone()) })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

fn link(capability: &str, answer: result::Value) -> Link {
    Link { capabilities: vec![capability.to_string()], sent: vec![], answer }
}

/// Each change is the request the runner's own socket takes, naming the
/// capability it needs, so an older runner refuses before it parses.
#[tokio::test]
async fn each_change_sends_the_request_the_socket_takes() {
    let mut l = link("person_access", result::Value::MemberList(rows()));
    let access = PeopleCmd::Access { handle: " bob ".into(), access: "viewer".into() };
    change_on(&mut l, access, false).await.expect("access");
    let limit = PeopleCmd::Limit { client_id: "ipad-o".into(), cap: "none".into() };
    change_on(&mut l, limit, true).await.expect("limit");
    let [first, second] = &l.sent[..] else { panic!("{:?}", l.sent) };
    assert_eq!(first.method, "member.set_access");
    assert!(matches!(&first.payload, Some(pb::request::Payload::MemberSetAccess(p)) if p.handle == "bob" && p.access == "viewer"));
    assert_eq!(second.method, "client.set_cap");
    assert!(matches!(&second.payload, Some(pb::request::Payload::ClientSetCap(p)) if p.client_id == "ipad-o" && p.cap.is_empty()), "none lifts the limit");
    assert_eq!(first.required_capabilities, vec!["person_access".to_string()]);
}

/// An approval carries the six digits and needs `step_up`; a runner without
/// the capability is told so without a round trip.
#[tokio::test]
async fn an_approval_names_the_key_by_its_digits() {
    let state = result::Value::StepUpKeyState(pb::StepUpKeyState { client_id: "phone-b".into(), state: "trusted".into(), code: String::new() });
    let mut l = link("step_up", state);
    let said = change_on(&mut l, PeopleCmd::Approve { client_id: "phone-b".into(), code: " 418207 ".into() }, false).await.expect("approve");
    assert_eq!(said, "phone-b's key is trusted");
    assert!(matches!(&l.sent[0].payload, Some(pb::request::Payload::ClientApproveStepUp(p)) if p.code == "418207"));
    let mut old = link("person_access", result::Value::MemberList(rows()));
    let refused = change_on(&mut old, PeopleCmd::Approve { client_id: "x".into(), code: "1".into() }, false).await;
    assert!(refused.is_err() && old.sent.is_empty(), "nothing was sent to a runner without step_up");
}
