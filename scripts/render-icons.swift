// Renders the lime emoji into assets/: icon-1024.png (app icon) and menubar.png (flat template, 44px = 22pt @2x).
import AppKit

func render(_ size: CGFloat) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size), bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = NSAttributedString(string: "🍋‍🟩", attributes: [.font: NSFont(name: "Apple Color Emoji", size: size * 0.8)!])
    let b = s.size()
    s.draw(at: NSPoint(x: (size - b.width) / 2, y: (size - b.height) / 2))
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let out = CommandLine.arguments[1]
try render(1024).representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(out)/icon-1024.png"))

// Flat: black lime with the white pith and segment lines punched out, drawn at 8x and scaled down so edges stay smooth.
let big = render(352)
for y in 0..<352 { for x in 0..<352 {
    let c = big.colorAt(x: x, y: y)!
    let a = c.alphaComponent > 0.5 && c.saturationComponent > 0.25 ? 1.0 : 0.0
    big.setColor(NSColor(deviceRed: 0, green: 0, blue: 0, alpha: a), atX: x, y: y)
}}
let small = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 44, pixelsHigh: 44, bitsPerSample: 8, samplesPerPixel: 4,
                             hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: small)
NSGraphicsContext.current!.imageInterpolation = .high
big.draw(in: NSRect(x: 0, y: 0, width: 44, height: 44))
NSGraphicsContext.restoreGraphicsState()
try small.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(out)/menubar.png"))
