import CoreGraphics
import Foundation
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
for window in windows where window[kCGWindowOwnerName as String] as? String == "Ov930 Native Workflow" {
    print(String(decoding: try JSONSerialization.data(withJSONObject: ["id": window[kCGWindowNumber as String]!, "bounds": window[kCGWindowBounds as String]!, "name": window[kCGWindowName as String] ?? ""], options: .sortedKeys), as: UTF8.self))
}
