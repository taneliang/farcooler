package com.farcooler.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.invisibleToUser
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

// The drawings behind the phone's first-run and empty states (ov-205), the
// iPhone's `FirstRunArt.swift`. Static and gray: they show the shape of what
// will appear, so the sentence beside them doesn't describe the screen. Nothing
// pulses, because that would read as loading.

@Composable
private fun fill() = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.12f)

/** Placeholder task rows: the shape tasks will take, not a loading state. */
@Composable
fun TaskSkeleton(modifier: Modifier = Modifier) {
    val fill = fill()
    Column(modifier.semantics { invisibleToUser() }, verticalArrangement = Arrangement.spacedBy(14.dp)) {
        listOf(0.72f, 0.54f, 0.63f).forEach { w ->
            Row(verticalAlignment = Alignment.Top) {
                Box(Modifier.padding(top = 2.dp).size(10.dp).border(1.5.dp, fill, CircleShape))
                Spacer(Modifier.width(16.dp))
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Box(Modifier.fillMaxWidth(w).height(8.dp).background(fill, RoundedCornerShape(4.dp)))
                    Box(Modifier.fillMaxWidth(w * 0.45f).height(6.dp).background(fill, RoundedCornerShape(3.dp)))
                }
            }
        }
    }
}

/** This phone, three dots, and a runner: "this reaches that" without a word. */
@Composable
fun PhoneOnboardingMark(modifier: Modifier = Modifier) {
    val tint = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier.semantics { invisibleToUser() },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Icon(Icons.Outlined.PhoneAndroid, contentDescription = null, tint = tint, modifier = Modifier.size(44.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            repeat(3) { Box(Modifier.size(3.dp).background(fill(), CircleShape)) }
        }
        Icon(Icons.Outlined.Dns, contentDescription = null, tint = tint, modifier = Modifier.size(44.dp))
    }
}
