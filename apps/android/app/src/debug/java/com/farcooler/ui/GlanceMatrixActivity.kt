package com.farcooler.ui

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.material3.Text
import com.farcooler.model.GlanceMark
import com.farcooler.model.GlanceMarkSize
import com.farcooler.model.GlancePalette
import com.farcooler.model.GlanceType

/**
 * §03's matrix, drawn — every state at every size, in both appearances.
 *
 * **Debug builds only.** It is declared in `src/debug/AndroidManifest.xml` and
 * compiled from `src/debug/java`, so nothing here reaches a release APK. It is
 * the direct counterpart of the `#if DEBUG #Preview("The matrix · every state,
 * every size")` at the bottom of iOS's `GlanceMark.swift`, and it exists for the
 * reason stated there: this is a drawing whose entire vocabulary is half-point
 * differences in line width, and the only way to check one of those is to put
 * all twelve beside each other at every size they are drawn at.
 *
 * An Activity rather than a `@Preview`, because a preview renders in Android
 * Studio's canvas and this has to be checkable from a terminal, on a device, in
 * a screenshot somebody else can look at:
 *
 * ```
 * adb shell am start -n com.farcooler.debug/com.farcooler.ui.GlanceMatrixActivity
 * ```
 *
 * **Look at it in monochrome too**, which is what a lock-screen accessory or a
 * watch face flattens it to. §03: "Nothing is redesigned for it: the mark
 * distinguishes states by stroke weight, fill and dash, so hue was always
 * redundant reinforcement." There is deliberately no greyscale block in here —
 * desaturating the SCREENSHOT is the same test and is one command, where a
 * `saveLayer` and a colour matrix inside the composable would be twenty lines of
 * scaffolding that could itself be wrong:
 *
 * ```
 * sips -s format png --matchTo /System/Library/ColorSync/Profiles/Generic\ Gray\ Profile.icc shot.png
 * ```
 *
 * **The light block is not decoration either.** §01's light palette is a
 * different set of values rather than a filter, and §09 flags it as the part of
 * the spec that is specified but never drawn: "Light mode is specified as values
 * but not drawn. Worth one pass before build." This is that pass, on this
 * platform.
 */
class GlanceMatrixActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { GlanceMatrix() }
    }
}

/**
 * Every combination of the three axes, in the order §03's table reads.
 *
 * Generated from the axes rather than listed, which is the same argument
 * `GlanceMark` makes for being three fields instead of a twelve-case enum: a
 * hand-written list can omit the combination nobody thought of, and that is
 * exactly the one that turns up on a phone.
 */
private val everyState: List<GlanceMark> =
    GlanceMark.Attention.entries.flatMap { attention ->
        GlanceMark.Core.entries.flatMap { core ->
            GlanceMark.Link.entries.map { GlanceMark(attention, core, it) }
        }
    }

@Composable
private fun GlanceMatrix() {
    Column(
        Modifier
            .fillMaxSize()
            .background(Color(GlancePalette.island.darkArgb))
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        Block("dark", dark = true, ground = Color(GlancePalette.widget.darkArgb))
        // The pale ground §01 names for the light case: "Surfaces invert to
        // black at 8% → 3%", which is a translucent black over whatever is
        // behind it, and what is behind it on a home screen is a wallpaper. A
        // near-white stands in for one here.
        Block("light", dark = false, ground = Color(0xFFF2F2F4))
    }
}

@Composable
private fun Block(title: String, dark: Boolean, ground: Color) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            title,
            style = glanceTextStyle(GlanceType.cardHeader),
            color = Color(GlancePalette.text1.darkArgb),
        )
        Box(Modifier.fillMaxWidth().background(ground).padding(10.dp)) {
            ProvideGlanceAppearance(dark) {
                Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    HeaderRow()
                    everyState.forEach { StateRow(it) }
                }
            }
        }
    }
}

/** The four diameters, named in mono because they are figures off a table. */
@Composable
private fun HeaderRow() {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.width(180.dp))
        GlanceMarkSize.entries.forEach { size ->
            Box(Modifier.width(44.dp), contentAlignment = Alignment.Center) {
                Text(
                    "${size.diameter.toInt()}",
                    style = glanceTextStyle(GlanceType.monoFigures),
                    color = glanceInk2(),
                )
            }
        }
    }
}

@Composable
private fun StateRow(mark: GlanceMark) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        // The row's own label is `phrase` — the same string TalkBack is given —
        // so what is spoken and what is drawn are checked against each other
        // here rather than in two places that can drift.
        Text(
            mark.phrase,
            style = glanceTextStyle(GlanceType.secondary),
            color = glanceInk2(),
            modifier = Modifier.width(180.dp),
        )
        GlanceMarkSize.entries.forEach { size ->
            Box(Modifier.width(44.dp), contentAlignment = Alignment.Center) {
                GlanceMarkView(mark, size, decorative = true)
            }
        }
    }
}
