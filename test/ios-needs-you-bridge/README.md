# Needs You static bridge regression

Run from the repository root after `cargo build -p farcooler-client`:

```sh
swiftc -module-cache-path /private/tmp/needs-you-module-cache \
  apps/shared/AgentKit/Sources/AgentKit/{NeedsYou,NeedsYouAggregate,NeedsYouWhose,PlanViewPresenter,WireTypes}.swift \
  test/ios-needs-you-bridge/main.swift target/debug/libfarcooler_client.a \
  -Xlinker -u -Xlinker _farcooler_client_plan_view -lc++ -o /private/tmp/needs-you-bridge-test
/private/tmp/needs-you-bridge-test
```

This runs the watch and extension adapter sources with the same forced static symbol link as their generated targets. It checks a populated list, another person's ask and duplicate-ID recovery. Target builds must also be checked for the symbol because this host executable does not run on a watch or extension.
