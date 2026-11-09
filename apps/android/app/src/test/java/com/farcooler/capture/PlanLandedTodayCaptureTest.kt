package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import com.farcooler.model.PlanReadState
import com.farcooler.ui.PlanFolds
import com.farcooler.ui.planItems
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Landed today, open, at the narrowest phone with a long real title (ov-1033): the section the shared card row draws
 * now that the phone's own row is gone. The board is the long one with a ticket landed an hour before its clock.
 * Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w320dp-h600dp-xhdpi")
class PlanLandedTodayCaptureTest {
    @Test fun landedTodayNarrow() = Capture.both("plan-landed-today-320") {
        val fixture = PlanFixtures.longWithLandedToday(alone = true)
        LazyColumn(Modifier.fillMaxSize()) {
            planItems(
                PlanReadState.Loaded(fixture.plan), fixture.statuses, onOpen = {}, onRetry = {},
                folds = PlanFolds(listOf("landed-today")) {}, needsYou = fixture.needsYou, themeColumns = 1,
            )
        }
    }
}
