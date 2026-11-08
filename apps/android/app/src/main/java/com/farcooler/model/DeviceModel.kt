package com.farcooler.model

/**
 * What this phone calls itself at pairing (ov-989), for the Mac's approve list: "Google Pixel 9 Pro",
 * "samsung SM-S928B" as the maker spells it. Its own word, which the runner keeps cleaned and doesn't check.
 */
object DeviceModel {
    /** The maker and the model, joined without saying the maker twice (`Build.MODEL` often starts with it). */
    fun describe(manufacturer: String?, model: String?): String {
        val maker = manufacturer.orEmpty().trim()
        val name = model.orEmpty().trim()
        return when {
            name.isEmpty() -> maker.replaceFirstChar { it.uppercase() }
            maker.isEmpty() || name.startsWith(maker, ignoreCase = true) -> name
            else -> "${maker.replaceFirstChar { it.uppercase() }} $name"
        }
    }

    fun current(): String = describe(android.os.Build.MANUFACTURER, android.os.Build.MODEL)
}
