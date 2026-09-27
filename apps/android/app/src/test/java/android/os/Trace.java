package android.os;

/**
 * A no-op `android.os.Trace` for the JVM unit tests, ahead of the SDK stub on
 * the test classpath.
 *
 * The SDK's `android.jar` is stubs that throw "not mocked", and Compose's
 * runtime opens a trace section around every composition. So without this,
 * `MinuteClockLifecycleTest` cannot compose anything on the plain JVM. Tracing
 * is instrumentation, not behavior: doing nothing here changes nothing a test
 * could observe. Narrower than `unitTests.isReturnDefaultValues`, which would
 * quietly default EVERY stub for every test in the module.
 */
public final class Trace {
    private Trace() {}

    public static boolean isEnabled() {
        return false;
    }

    public static void beginSection(String sectionName) {}

    public static void endSection() {}

    public static void beginAsyncSection(String methodName, int cookie) {}

    public static void endAsyncSection(String methodName, int cookie) {}

    public static void setCounter(String counterName, long counterValue) {}
}
