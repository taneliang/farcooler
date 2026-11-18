# Workflow capacity

The daemon shares its agent session limit across all workspaces. By default it runs at most four builds and agent reviews at once. To change the limit, create `workflow/workflow-places.json` under the daemon's root directory, for example:

```json
{"agent": 3}
```

The value must be a positive integer. Restart the daemon to load a change. `workflow get` reports the effective limit, occupied places, and queued effects under `places`. Landing group operations and other work have separate limits, so a full agent pool does not prevent them from starting.
