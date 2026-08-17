import AppKit
import CoreImage

/// A QR code, drawn large enough to scan off a screen.
///
/// CoreImage renders one pixel per module. On a Mac that is a postage stamp
/// somewhere in the top-left of the view and unscannable across a desk, so the
/// transform is not cosmetic — it is the difference between a feature and a
/// picture of one.
///
/// Nearest-neighbor is the other half of it. A QR code is a grid of hard edges
/// and the default sampling smooths them, which is exactly the wrong thing to
/// do to something a camera has to threshold. `CISampleNearest` before the
/// scale keeps the modules square.
func qrImage(_ payload: String, scale: CGFloat = 12) -> NSImage? {
    guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
    filter.setValue(Data(payload.utf8), forKey: "inputMessage")
    // Medium correction: the code is read off a lit screen at close range, not
    // off a printed label that may be scuffed, so capacity is worth more than
    // redundancy. The ceremony's byte budget is stated against this level.
    filter.setValue("M", forKey: "inputCorrectionLevel")
    guard let output = filter.outputImage else { return nil }

    let sharp = output
        .samplingNearest()
        .transformed(by: CGAffineTransform(scaleX: scale, y: scale))
    guard let cg = CIContext().createCGImage(sharp, from: sharp.extent) else { return nil }
    return NSImage(cgImage: cg, size: NSSize(width: sharp.extent.width, height: sharp.extent.height))
}

/// The largest payload this build will put on screen without complaint.
///
/// Not a limit this enforces — `farcooler_client_ceremony_reply` refuses an
/// oversized manifest in Rust, where the rule belongs, and answers `too_large`.
/// This is what the app passes it as its own encoder's honest answer: a
/// version-40 code at medium correction holds 2331 binary bytes, and a code
/// that large on a laptop screen is at the edge of what a phone camera reads
/// across a desk. Staying well under it is what keeps the scan a one-second
/// affair rather than a minute of waving a phone about.
let codeBudgetBytes = 1_800
