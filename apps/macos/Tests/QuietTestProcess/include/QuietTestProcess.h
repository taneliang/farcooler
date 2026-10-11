#ifndef QUIET_TEST_PROCESS_H
#define QUIET_TEST_PROCESS_H

/// Makes the test process one that can never be activated: no Dock icon, no
/// menu bar, and no taking the frontmost spot from the person's app (ov-473).
/// It runs once by itself when the test bundle loads; calling it again is
/// harmless, and `TestWindow` does, so the linker keeps this file.
void farcooler_quiet_test_process(void);

#endif
