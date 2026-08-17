import CoreImage
import SwiftUI
import UIKit

/// A QR code, drawn large enough to scan off a screen.
///
/// CoreImage renders at one pixel per module, which is a postage stamp on a
/// phone and unscannable across a desk. The transform is not cosmetic.
///
/// Nothing here decides anything. The string handed in came from
/// `farcooler_client_ceremony_offer` or `farcooler_client_ceremony_reply`, and
/// this file's whole job is to make it visible to another camera.
func qrImage(_ payload: String) -> UIImage? {
    guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
    filter.setValue(Data(payload.utf8), forKey: "inputMessage")
    // Medium correction: the code is read off a lit screen at close range,
    // not off a printed label, so capacity is worth more than redundancy.
    filter.setValue("M", forKey: "inputCorrectionLevel")
    guard let output = filter.outputImage else { return nil }
    let scaled = output.transformed(by: CGAffineTransform(scaleX: 12, y: 12))
    let context = CIContext()
    guard let cg = context.createCGImage(scaled, from: scaled.extent) else { return nil }
    return UIImage(cgImage: cg)
}

/// The code, filling whatever space a screen gives it.
///
/// `.interpolation(.none)` is the reason this exists rather than a bare
/// `Image`: SwiftUI smooths an upscaled bitmap by default, and a blurred module
/// edge is what makes a code that looks fine to a person fail to decode for a
/// camera.
struct CodeImageView: View {
    let payload: String

    var body: some View {
        if let image = qrImage(payload) {
            Image(uiImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .padding(12)
                // White behind the code, whatever the app's theme is doing.
                // A dark ground under a dark-on-clear code is not a styling
                // problem, it is a code no camera can read.
                .background(Color.white)
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .accessibilityLabel("A code for another device to scan")
        } else {
            // No raw error, and nothing pretending a code is on screen.
            Text("This code couldn’t be drawn.")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }
}
