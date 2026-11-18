package com.farcooler.ui

import android.os.Bundle
import android.util.Base64
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/** Debug-only host: the card extra is a base64-encoded Rust golden presentation. */
class WorkflowFixtureActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val json = String(Base64.decode(requireNotNull(intent.getStringExtra("card")), Base64.DEFAULT))
        val run = WorkflowPresentations.decode(json, "workspace").runs.single()
        setContent {
            FarCoolerTheme {
                Surface {
                    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
                        WorkflowRunCard(run, enabled = true, detailsOpen = intent.getBooleanExtra("expanded", false)) {}
                    }
                }
            }
        }
    }
}
