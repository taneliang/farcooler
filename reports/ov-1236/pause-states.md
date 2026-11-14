# Builder pause states (ov-1236)

A builder is stopped for two independent reasons: a landing gate is running (gate), or the orchestrator paused it by hand (manual). It is continued only when neither remains.

| State | Gate starts | Gate ends | Manual pause | Manual resume |
| --- | --- | --- | --- | --- |
| None (running) | Stops it → gate | Nothing | Stops it → manual | Nothing |
| Gate | Stays stopped (more gates) | Continues it once the last gate ends → none | Marks manual; already stopped → both | Nothing |
| Manual | Marks the gate; already stopped → both | Skips it; stays stopped | Nothing | Continues it → none |
| Both | Stays stopped | Last gate ends: stays stopped → manual | Nothing | Stays stopped → gate (the gate's sweep keeps it stopped) |
