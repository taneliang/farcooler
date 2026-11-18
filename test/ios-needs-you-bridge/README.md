# Needs You static bridge regression

Run from the repository root after `cargo build -p farcooler-client`:

```sh
swiftc -module-cache-path /private/tmp/needs-you-module-cache \
  apps/shared/AgentKit/Sources/AgentKit/{NeedsYou,NeedsYouAggregate,NeedsYouWhose,PlanViewPresenter,WireTypes}.swift \
  test/ios-needs-you-bridge/main.swift target/debug/libfarcooler_client.a \
  -Xlinker -u -Xlinker _farcooler_client_plan_view -lc++ -o /private/tmp/needs-you-bridge-test
/private/tmp/needs-you-bridge-test
```

This runs the phone adapter sources with the static Rust client. The watch and extensions compile the data types without the adapter or Rust client. It checks a populated list, another person's ask and duplicate-ID recovery.
