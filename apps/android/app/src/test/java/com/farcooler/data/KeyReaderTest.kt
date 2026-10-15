package com.farcooler.data

import java.io.IOException
import java.security.UnrecoverableKeyException
import javax.crypto.AEADBadTagException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** A failed key read must never be mistaken for a key that can never be read (ov-572). */
class KeyReaderTest {
    @Test
    fun transientFailureOnceThenWorks_returnsTheKey() {
        var calls = 0
        val result = KeyReader.read(pause = {}) {
            if (calls++ == 0) throw IOException("keystore busy")
            "the-key"
        }
        assertEquals(KeyRead.Found("the-key"), result)
        assertEquals(2, calls)
    }

    @Test
    fun transientFailureThatPersists_isFailedNotUnusable() {
        var calls = 0
        val result = KeyReader.read(pause = {}) { calls++; throw IOException("keystore busy") }
        assertTrue(result is KeyRead.Failed)
        assertEquals(3, calls)
    }

    @Test
    fun permanentFailures_areUnusableWithoutRetry() {
        listOf<Exception>(
            KeyEntryMissing(),
            UnrecoverableKeyException("invalidated"),
            AEADBadTagException("wrong key"),
            IllegalArgumentException("Illegal base64 character"),
            RuntimeException("wrapped", UnrecoverableKeyException()),
        ).forEach { error ->
            var calls = 0
            val result = KeyReader.read(pause = {}) { calls++; throw error }
            assertEquals(KeyRead.Unusable, result)
            assertEquals(1, calls)
        }
    }

    @Test
    fun failedRead_keepsTheKeyAndReportsAnError() {
        var forgot = false
        var reported: com.farcooler.model.Trouble? = null
        val result = KeyReader.read(pause = {}) { throw IOException("keystore busy") }
        KeyReader.settle(result, current = true, forget = { forgot = true }, report = { reported = it })
        assertEquals(false, forgot)
        assertTrue(reported != null)
    }

    @Test
    fun unusableCurrentKey_isForgottenAndReported_otherSlotsKept() {
        var forgot = 0
        var reports = 0
        KeyReader.settle(KeyRead.Unusable, current = true, forget = { forgot++ }, report = { reports++ })
        KeyReader.settle(KeyRead.Unusable, current = false, forget = { forgot++ }, report = { reports++ })
        assertEquals(1, forgot)
        assertEquals(1, reports)
    }
}
