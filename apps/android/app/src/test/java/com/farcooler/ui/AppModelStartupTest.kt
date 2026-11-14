package com.farcooler.ui

import android.content.Context
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModelStore
import androidx.test.platform.app.InstrumentationRegistry
import com.farcooler.FarCoolerApp
import com.farcooler.data.Identity
import com.farcooler.data.NodeIdentity
import com.farcooler.data.Themes
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.junit.runners.model.FrameworkMethod
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.internal.bytecode.InstrumentationConfiguration

/** A cold process with a saved runner must construct the real model before its first screen (ov-1180). */
@RunWith(AppStartupTestRunner::class)
@Config(sdk = [33], manifest = Config.NONE, application = FarCoolerApp::class)
@OptIn(ExperimentalCoroutinesApi::class)
class AppModelStartupTest {
    @Test
    fun savedRunnerCanStartWhileTheModelIsBeingConstructed() {
        val application = RuntimeEnvironment.getApplication() as FarCoolerApp
        application.getSharedPreferences("farcooler.hosts", Context.MODE_PRIVATE).edit()
            .putString("hosts", """[{"id":"startup-test","label":"Offline runner","address":"127.0.0.1","port":1,"user":"test"}]""")
            .commit()

        // Main.immediate matters: delaying the fleet's collection until after
        // construction would hide the original read-before-initialize crash.
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            runTest {
                val store = ViewModelStore()
                Dispatchers.setMain(UnconfinedTestDispatcher(testScheduler))
                try {
                    val model = AppModel(application, SavedStateHandle())
                    store.put("model", model)
                    assertEquals("startup-test", model.hosts.hosts.value.single().id)
                    assertNotNull(model.fleet.connection("startup-test"))
                } finally {
                    store.clear()
                    Dispatchers.resetMain()
                }
            }
        }
    }

    @Test
    fun everyGlobalStoreIsReadyBeforeAnActivityOrModelExists() {
        RuntimeEnvironment.getApplication()
        // Reads that need preferences but do not mint keys or need a native core.
        assertEquals(emptySet<String>(), Identity.runnersOnPrevious)
        assertNull(NodeIdentity.storedPrivateKey)
        Themes.select("startup-test-theme")
        val prefs = RuntimeEnvironment.getApplication()
            .getSharedPreferences("farcooler", Context.MODE_PRIVATE)
        assertEquals("startup-test-theme", prefs.getString("app.theme", null))
        // Initializing Themes before the model must also load the native
        // catalog, rather than freezing the process on its one fallback.
        if (com.farcooler.core.coreIsAvailable) {
            assertTrue("Built-in themes were not loaded", Themes.available.value.size > 1)
        }
    }
}

/** JNI libraries belong to one class loader, shared with the ordinary JVM native tests. */
class AppStartupTestRunner(testClass: Class<*>) : RobolectricTestRunner(testClass) {
    override fun createClassLoaderConfig(method: FrameworkMethod): InstrumentationConfiguration =
        InstrumentationConfiguration.Builder(super.createClassLoaderConfig(method))
            .doNotAcquireClass("com.farcooler.core.NativeKt")
            .doNotAcquireClass("com.farcooler.core.NativeLibrary")
            .doNotAcquireClass("com.farcooler.core.NativeClient")
            .doNotAcquireClass("com.farcooler.core.NativeVt")
            .build()
}
