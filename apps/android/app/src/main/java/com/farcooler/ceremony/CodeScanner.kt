package com.farcooler.ceremony

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import android.util.Log
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.AspectRatioStrategy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.lifecycle.awaitInstance
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.net.rethrowIfCancellation
import com.farcooler.ui.Radius
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.LuminanceSource
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The camera, and the first code it reads.
 *
 * ONE hit and it is done. A scanner that keeps firing is a scanner that reads
 * the second code in the frame — two phones on a desk during onboarding is the
 * ordinary case, not the unlucky one — and the ceremony's whole shape is one
 * code answering one other.
 *
 * This class decides nothing about what it read. The payload goes to
 * `CeremonyStore`, which hands it to Rust.
 */
class CodeScanner {
    private val _scanned = MutableStateFlow<String?>(null)

    /** The payload of the first code read since [start]. */
    val scanned: StateFlow<String?> = _scanned.asStateFlow()

    /**
     * Armed rather than a "have I published yet" check on [scanned], because
     * the analyzer runs on its own thread and two frames can carry the same
     * code a few milliseconds apart. A compare-and-set is what makes "the
     * first" mean one.
     */
    private val armed = AtomicBoolean(false)

    private val _cameraFailed = MutableStateFlow(false)

    /**
     * The camera didn't start: no provider, or it wouldn't bind. Without this
     * the preview was a black square that never read anything and never said
     * why (ov-180).
     */
    val cameraFailed: StateFlow<Boolean> = _cameraFailed.asStateFlow()

    internal fun cameraUnavailable() {
        _cameraFailed.value = true
    }

    /** Run the camera setup in [block]; if it throws, the screen says the camera didn't start. */
    internal fun bindCamera(block: () -> Unit) {
        try {
            block()
        } catch (e: Exception) {
            cameraUnavailable()
        }
    }

    /** Begin, or begin again after a refusal. */
    fun start() {
        _cameraFailed.value = false
        _scanned.value = null
        armed.set(true)
    }

    fun stop() {
        armed.set(false)
    }

    internal fun report(payload: String) {
        if (armed.compareAndSet(true, false)) _scanned.value = payload
    }
}

/** The scan screen's sentences about the camera, worded for whether pasting a key is open here. */
internal object ScanCopy {
    private fun orPaste(canPaste: Boolean) =
        if (canPaste) ", or add this device by pasting its key instead." else "."

    fun cameraFailed(canPaste: Boolean) =
        "The camera didn’t start. Another app may be using it. Go back and try again" +
            orPaste(canPaste)

    fun cameraOff(canPaste: Boolean) =
        "The camera is turned off for Far Cooler. You can turn it on in Settings" + // casing ok: Android's Settings app
            orPaste(canPaste)
}

/**
 * Point the camera at a code, having said why the camera is on.
 *
 * The permission is requested WHEN THIS SCREEN OPENS and never at launch. A
 * camera prompt at first run has nothing to explain it; here the sentence
 * underneath is on screen while the system asks, and it is the same sentence
 * either way.
 */
@Composable
fun ScanScreen(
    scanner: CodeScanner,
    instruction: String,
    onCancel: () -> Unit,
    /** False on the device that's already signed in: only the device being added can paste its key. */
    canPaste: Boolean = true,
) {
    val context = LocalContext.current
    var granted by remember {
        mutableStateOf(
            context.checkSelfPermission(Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED
        )
    }
    var asked by remember { mutableStateOf(false) }
    val request = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { allowed ->
        granted = allowed
        asked = true
    }

    LaunchedEffect(Unit) {
        scanner.start()
        if (!granted) request.launch(Manifest.permission.CAMERA)
    }

    Column(
        Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        if (granted) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .aspectRatio(1f)
                    .clip(RoundedCornerShape(Radius.medium))
            ) {
                CameraPreview(scanner, Modifier.fillMaxSize())
            }
            Spacer(Modifier.height(18.dp))
            val cameraFailed by scanner.cameraFailed.collectAsStateWithLifecycle()
            Text(
                if (cameraFailed) {
                    ScanCopy.cameraFailed(canPaste)
                } else {
                    instruction
                },
                style = MaterialTheme.typography.bodyMedium,
                color = if (cameraFailed) MaterialTheme.colorScheme.error
                else MaterialTheme.colorScheme.onSurface,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(18.dp))
            TextButton(onClick = onCancel) { Text("Cancel") }
        } else {
            Text(
                "Far Cooler uses the camera to scan the code on a device you’re adding.",
                style = MaterialTheme.typography.bodyMedium,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(18.dp))
            if (asked) {
                // Denied. Android will not ask twice, so the only honest button
                // is the one that goes where the answer can be changed — and
                // the manual path is still there, which is what the second
                // sentence says.
                Text(
                    ScanCopy.cameraOff(canPaste),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(18.dp))
                Button(onClick = {
                    context.startActivity(
                        Intent(
                            Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                            Uri.fromParts("package", context.packageName, null),
                        )
                    )
                }) { Text("Open settings") }
                Spacer(Modifier.height(8.dp))
            }
            TextButton(onClick = onCancel) { Text("Cancel") }
        }
    }
}

/**
 * The preview, and the frames the decoder looks at.
 *
 * Unbound when this leaves the composition. `bindToLifecycle` ties the camera
 * to the ACTIVITY's lifecycle, which outlives this screen — so without the
 * explicit unbind the torch stays on the moment someone navigates away, which
 * is both a battery bug and the kind of thing a person notices about a camera.
 */
@Composable
private fun CameraPreview(scanner: CodeScanner, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val view = remember {
        PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER }
    }
    val reader = remember { QrReader() }
    var provider by remember { mutableStateOf<ProcessCameraProvider?>(null) }

    LaunchedEffect(Unit) {
        provider = try {
            ProcessCameraProvider.awaitInstance(context)
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            scanner.cameraUnavailable()
            null
        }
    }

    DisposableEffect(provider) {
        val release = AnalysisThread.attach(
            camera = provider,
            bind = { camera, executor ->
                val preview = Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
                val analysis = ImageAnalysis.Builder()
                    // The newest frame, and no queue. A backlog of frames is a
                    // backlog of decodes of a code that has already been read.
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .setResolutionSelector(ANALYSIS_RESOLUTION)
                    .build()
                var frames = 0
                analysis.setAnalyzer(executor) { image ->
                    image.use { frame ->
                        // One line per binding, so `adb logcat -s FarCoolerScan`
                        // says whether frames arrive at all and at what size —
                        // the question ov-876 could not answer from a phone.
                        if (frames++ == 0) {
                            Log.i(SCAN_LOG, "first frame ${frame.width}x${frame.height}")
                        }
                        reader.read(frame)?.let(scanner::report)
                    }
                }
                scanner.bindCamera {
                    camera.unbindAll()
                    camera.bindToLifecycle(
                        lifecycleOwner,
                        CameraSelector.DEFAULT_BACK_CAMERA,
                        preview,
                        analysis,
                    )
                }
            },
            unbind = { camera -> camera.unbindAll() },
        )
        onDispose(release)
    }

    AndroidView(factory = { view }, modifier = modifier)
}

/** The logcat tag for the scanner's own lines. */
private const val SCAN_LOG = "FarCoolerScan"

/**
 * The analysis frame CameraX is asked for: 1280×960, 4:3 like the sensor.
 *
 * CameraX's own default is 640×480, and that is too coarse for the code the
 * other device shows. A reply carrying one tunneled runner is about 660 bytes,
 * which the Mac draws as a version-19 code — 89 modules plus its one-module
 * border. Held so the code is 40% of the frame's height, 480 rows give it
 * about two camera pixels per module, under what ZXing can threshold; 960
 * rows give it four. `QrDensityTest` measures both, with the Mac's own render.
 *
 * "Closest higher then lower": a camera that has no 1280×960 stream picks the
 * nearest one above it before settling for one below.
 */
private val ANALYSIS_RESOLUTION: ResolutionSelector = ResolutionSelector.Builder()
    .setAspectRatioStrategy(AspectRatioStrategy.RATIO_4_3_FALLBACK_AUTO_STRATEGY)
    .setResolutionStrategy(
        ResolutionStrategy(
            Size(1280, 960),
            ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER,
        )
    )
    .build()

/**
 * The thread the analyzer runs on, made for ONE camera binding and shut down
 * with it.
 *
 * This is ov-876. The thread used to be `remember`ed once for the screen while
 * its shutdown sat in the `onDispose` of the effect keyed on the camera
 * provider. That effect runs first with no provider, and is disposed the
 * moment the provider arrives — so the thread was shut down a few milliseconds
 * after the screen opened, before any camera was bound. CameraX then handed
 * every frame to an executor that refused it, dropped the frame quietly, and
 * the preview kept running over a scanner that never saw a single frame, at
 * any distance. Owning the thread here, per binding, leaves no second lifetime
 * to get out of step.
 */
internal object AnalysisThread {
    /**
     * Bind [camera], if there is one yet, on a fresh thread; the returned
     * function unbinds it and stops that thread. Exactly what one run of a
     * keyed effect does, and its `onDispose`.
     */
    fun <C : Any> attach(
        camera: C?,
        bind: (C, ExecutorService) -> Unit,
        unbind: (C) -> Unit,
    ): () -> Unit {
        if (camera == null) return {}
        // One thread, named, for the same reason the client core has one: a
        // native decode on an anonymous pool thread tells you nothing in a trace.
        val executor = Executors.newSingleThreadExecutor { runnable ->
            Thread(runnable, "farcooler-scan").apply { isDaemon = true }
        }
        bind(camera, executor)
        return {
            unbind(camera)
            executor.shutdown()
        }
    }
}

/**
 * One QR code out of one camera frame, with ZXing.
 *
 * The luminance plane is used as it arrives — a QR decoder wants brightness and
 * nothing else, so there is no color conversion and no bitmap in the middle of
 * this. The buffer is reused across frames because this runs thirty times a
 * second and a fresh two-megabyte array each time is thirty a second for the
 * collector.
 */
internal class QrReader {
    private val reader = MultiFormatReader().apply {
        setHints(
            mapOf(
                DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
                // Worth it here: this runs on a preview frame of a code on a
                // screen, where a reflection or a slight angle is ordinary, and
                // the cost is a few milliseconds on frames that would otherwise
                // simply fail.
                DecodeHintType.TRY_HARDER to true,
            )
        )
    }

    private var luminance = ByteArray(0)

    private val unexpected = mutableSetOf<String>()

    fun read(image: ImageProxy): String? {
        val plane = image.planes.firstOrNull() ?: return null
        val stride = plane.rowStride
        val needed = stride * image.height
        // Sized by the stride rather than the width: a camera is free to pad
        // every row, and `PlanarYUVLuminanceSource` refuses an array shorter
        // than the geometry it was given.
        if (luminance.size != needed) luminance = ByteArray(needed)
        val buffer = plane.buffer
        buffer.rewind()
        buffer.get(luminance, 0, minOf(buffer.remaining(), needed))
        return read(luminance, stride, image.width, image.height)
    }

    /** The decode itself, on a luminance plane rows of [stride] bytes apart. */
    fun read(luminance: ByteArray, stride: Int, width: Int, height: Int): String? {
        val source = PlanarYUVLuminanceSource(
            luminance,
            stride,
            height,
            0,
            0,
            width,
            height,
            false,
        )
        // The inverted pass is for a code shown light-on-dark by something else.
        // Far Cooler always draws black on white — see `CodeImage` — but the
        // device on the other side of this exchange may not be Far Cooler's
        // current version.
        return decode(source) ?: decode(source.invert())
    }

    private fun decode(source: LuminanceSource): String? {
        val result = try {
            reader.decodeWithState(BinaryBitmap(HybridBinarizer(source)))
        } catch (e: ReaderException) {
            null
        } catch (e: RuntimeException) {
            // Not "no code here": ZXing failing, or a frame shaped unlike what
            // it was told. Said once per kind rather than thirty times a second.
            if (unexpected.add(e.javaClass.name)) {
                Log.w(SCAN_LOG, "decode failed", e)
            }
            null
        }
        // Not finding a code in a frame is the ordinary answer, thirty times a
        // second, and ZXing reports it by throwing. Reset either way, or the
        // reader carries state from a frame into the next one.
        reader.reset()
        return result?.text?.takeIf { it.isNotEmpty() }
    }
}
