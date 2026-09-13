// Android status-bar notifications require a white silhouette on a transparent canvas.
// This uses the mast, foot and sail geometry of assets/icon-ios.svg, without its app-tile background.
import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
for (density, size) in [("ldpi", 18), ("mdpi", 24), ("hdpi", 36), ("xhdpi", 48), ("xxhdpi", 72), ("xxxhdpi", 96)] {
    let context = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: size * 4,
        space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.translateBy(x: 0, y: CGFloat(size)); context.scaleBy(x: CGFloat(size) / 76, y: -CGFloat(size) / 76)
    context.translateBy(x: -13, y: -12)
    context.setFillColor(CGColor(gray: 1, alpha: 1))
    context.addPath(CGPath(roundedRect: CGRect(x: 29.5, y: 20, width: 6, height: 62), cornerWidth: 3, cornerHeight: 3, transform: nil)); context.fillPath()
    context.addPath(CGPath(roundedRect: CGRect(x: 29.5, y: 80, width: 25, height: 6.5), cornerWidth: 3.25, cornerHeight: 3.25, transform: nil)); context.fillPath()
    context.move(to: CGPoint(x: 40, y: 24)); context.addQuadCurve(to: CGPoint(x: 76, y: 51), control: CGPoint(x: 67, y: 30))
    context.addQuadCurve(to: CGPoint(x: 40, y: 74), control: CGPoint(x: 65, y: 70)); context.closePath(); context.fillPath()
    let directory = root.appendingPathComponent("plugins/remote/android/src/main/res/drawable-\(density)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let destination = CGImageDestinationCreateWithURL(directory.appendingPathComponent("push_small.png") as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(destination, context.makeImage()!, nil)
    guard CGImageDestinationFinalize(destination) else { fatalError("Notification icon export failed") }
}
