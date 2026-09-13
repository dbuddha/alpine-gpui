import AppKit
import CoreGraphics
import Foundation

// Draws the Alpine Editor icon at one size. macOS Big Sur and later expect a
// rounded square filling most of the canvas, so the artwork is inset rather
// than bled to the edge.
func drawIcon(size: CGFloat) -> CGImage? {
    let scale: CGFloat = size / 1024.0
    guard let space = CGColorSpace(name: CGColorSpace.sRGB),
          let ctx = CGContext(
            data: nil, width: Int(size), height: Int(size),
            bitsPerComponent: 8, bytesPerRow: 0, space: space,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return nil }

    ctx.interpolationQuality = .high
    ctx.setAllowsAntialiasing(true)

    // Rounded-square body, inset to the platform's icon grid.
    let inset: CGFloat = 100 * scale
    let body = CGRect(x: inset, y: inset, width: size - inset * 2, height: size - inset * 2)
    let corner: CGFloat = 185 * scale
    let squircle = CGPath(roundedRect: body, cornerWidth: corner, cornerHeight: corner,
                          transform: nil)

    ctx.saveGState()
    ctx.addPath(squircle)
    ctx.clip()
    // Slate gradient, a shade above the editor background so the icon reads as
    // an object rather than a hole.
    let colours = [
        CGColor(srgbRed: 0.16, green: 0.18, blue: 0.22, alpha: 1),
        CGColor(srgbRed: 0.09, green: 0.10, blue: 0.12, alpha: 1),
    ] as CFArray
    if let gradient = CGGradient(colorsSpace: space, colors: colours, locations: [0, 1]) {
        ctx.drawLinearGradient(
            gradient, start: CGPoint(x: body.minX, y: body.maxY),
            end: CGPoint(x: body.maxX, y: body.minY), options: [])
    }

    // Two peaks. The far peak is cooler and dimmer to give depth at 512px
    // without relying on detail that vanishes at 32px.
    let base = body.minY + body.height * 0.30
    ctx.setFillColor(CGColor(srgbRed: 0.29, green: 0.42, blue: 0.58, alpha: 1))
    ctx.beginPath()
    ctx.move(to: CGPoint(x: body.minX + body.width * 0.34, y: base))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.62, y: body.minY + body.height * 0.74))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.90, y: base))
    ctx.closePath()
    ctx.fillPath()

    ctx.setFillColor(CGColor(srgbRed: 0.55, green: 0.72, blue: 0.90, alpha: 1))
    ctx.beginPath()
    ctx.move(to: CGPoint(x: body.minX + body.width * 0.10, y: base))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.40, y: body.minY + body.height * 0.84))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.70, y: base))
    ctx.closePath()
    ctx.fillPath()

    // Snow cap on the near peak, clipped to the peak's own silhouette.
    ctx.saveGState()
    ctx.beginPath()
    ctx.move(to: CGPoint(x: body.minX + body.width * 0.10, y: base))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.40, y: body.minY + body.height * 0.84))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.70, y: base))
    ctx.closePath()
    ctx.clip()
    ctx.setFillColor(CGColor(srgbRed: 0.96, green: 0.97, blue: 0.99, alpha: 1))
    ctx.beginPath()
    ctx.move(to: CGPoint(x: body.minX + body.width * 0.29, y: body.minY + body.height * 0.70))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.40, y: body.minY + body.height * 0.84))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.51, y: body.minY + body.height * 0.70))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.44, y: body.minY + body.height * 0.72))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.38, y: body.minY + body.height * 0.68))
    ctx.addLine(to: CGPoint(x: body.minX + body.width * 0.34, y: body.minY + body.height * 0.72))
    ctx.closePath()
    ctx.fillPath()
    ctx.restoreGState()

    // Caret, the one mark that says "editor" rather than "outdoors".
    // Sits below the ridge line rather than through it, so it reads as a
    // caret on a line of text instead of a pole planted in the mountain.
    let caretWidth = body.width * 0.052
    let caretHeight = body.height * 0.17
    let caret = CGRect(
        x: body.midX - caretWidth / 2, y: body.minY + body.height * 0.095,
        width: caretWidth, height: caretHeight)
    ctx.setFillColor(CGColor(srgbRed: 0.98, green: 0.78, blue: 0.35, alpha: 1))
    ctx.addPath(CGPath(roundedRect: caret, cornerWidth: caretWidth / 2,
                       cornerHeight: caretWidth / 2, transform: nil))
    ctx.fillPath()

    ctx.restoreGState()
    return ctx.makeImage()
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write("icon generation failed: \(message)\n".data(using: .utf8)!)
    exit(1)
}

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "/tmp/AlpineEditor.iconset"
do {
    try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
} catch {
    fail("could not create \(out): \(error)")
}
// The exact set `iconutil` requires.
let plan: [(Int, String)] = [
    (16, "icon_16x16"), (32, "icon_16x16@2x"), (32, "icon_32x32"), (64, "icon_32x32@2x"),
    (128, "icon_128x128"), (256, "icon_128x128@2x"), (256, "icon_256x256"),
    (512, "icon_256x256@2x"), (512, "icon_512x512"), (1024, "icon_512x512@2x"),
]
// A silently partial iconset produces a bad bundle that only fails much
// later, so every step here is fatal.
for (pixels, name) in plan {
    guard let image = drawIcon(size: CGFloat(pixels)) else { fail("could not draw \(name)") }
    let rep = NSBitmapImageRep(cgImage: image)
    rep.size = NSSize(width: pixels, height: pixels)
    guard let data = rep.representation(using: .png, properties: [:]) else {
        fail("could not encode \(name)")
    }
    do {
        try data.write(to: URL(fileURLWithPath: "\(out)/\(name).png"))
    } catch {
        fail("could not write \(name): \(error)")
    }
}
print("wrote iconset to \(out)")
