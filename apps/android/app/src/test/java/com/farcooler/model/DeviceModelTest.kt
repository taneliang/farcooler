package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/** What a phone says it is at pairing (ov-989). */
class DeviceModelTest {
    @Test fun `the maker is said once`() {
        assertEquals("Google Pixel 9 Pro", DeviceModel.describe("Google", "Pixel 9 Pro"))
        assertEquals("Samsung SM-S928B", DeviceModel.describe("samsung", "SM-S928B"))
        assertEquals("OnePlus 12", DeviceModel.describe("OnePlus", "OnePlus 12"))
    }

    @Test fun `a missing half is not a blank`() {
        assertEquals("Pixel 9", DeviceModel.describe("", "Pixel 9"))
        assertEquals("Google", DeviceModel.describe("google", null))
        assertEquals("", DeviceModel.describe(null, null))
    }
}
