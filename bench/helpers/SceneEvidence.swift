import CoreGraphics
import CoreVideo
import Foundation

let sceneSchema = "alpine-scene-diagnostic/v1"
let sceneFrameCap = 2048
let sceneExpectedA: [UInt8] = [0, 118, 188, 225, 255]
let sceneExpectedB = Array(sceneExpectedA.reversed())
let sceneTolerance = 12

func sceneState(_ samples: [UInt8]) -> String {
    guard samples.count == 135 else { return "invalid" }
    for (name, expected) in [("A", sceneExpectedA), ("B", sceneExpectedB)] {
        if samples.enumerated().allSatisfy({ index, value in
            abs(Int(value) - Int(expected[index / 27])) <= sceneTolerance
        }) { return name }
    }
    return "invalid"
}

struct SceneSampler {
    let width: Int
    let height: Int
    let region: CGRect
    let points: [(Int, Int)]

    init(width: Int, height: Int, region: CGRect, scale: CGFloat) throws {
        let values = [region.minX, region.minY, region.width, region.height, scale]
        guard values.allSatisfy({ $0.isFinite }), scale >= 1, scale <= 4,
              width > 0, height > 0, width <= 4096, height <= 4096,
              width * height <= 4_194_304 else {
            throw HelperFailure.usage("diagnostic scene dimensions or scale out of range")
        }
        let scaled = CGRect(x: region.minX * scale, y: region.minY * scale,
                            width: region.width * scale, height: region.height * scale)
        guard [scaled.minX, scaled.minY, scaled.width, scaled.height].allSatisfy({ $0.rounded() == $0 }),
              scaled.minX >= 0, scaled.minY >= 0, scaled.width >= 50, scaled.height >= 12,
              scaled.maxX <= CGFloat(width), scaled.maxY <= CGFloat(height) else {
            throw HelperFailure.usage("diagnostic scene region must be integral, complete and in bounds")
        }
        self.width = width
        self.height = height
        self.region = scaled
        var locations: [(Int, Int)] = []
        locations.reserveCapacity(45)
        for patch in 0..<5 {
            for row in 1...3 {
                for column in 1...3 {
                    let x = scaled.minX + (CGFloat(patch) + CGFloat(column) / 4) * scaled.width / 5
                    let y = scaled.minY + CGFloat(row) * scaled.height / 4
                    locations.append((Int(x), Int(y)))
                }
            }
        }
        points = locations
    }

    func validateTransform(rect: CGRect, contentScale: Double,
                           displayScale: Double, requestedScale: Double) throws {
        let values = [Double(rect.minX), Double(rect.minY), Double(rect.width),
                      Double(rect.height), contentScale, displayScale, requestedScale]
        guard values.allSatisfy({ $0.isFinite }), contentScale > 0,
              displayScale >= 1, displayScale <= 4, rect.minX == 0, rect.minY == 0,
              abs(contentScale * displayScale - requestedScale) <= 0.000_001,
              abs(Double(rect.width) * displayScale - Double(width)) <= 0.000_001,
              abs(Double(rect.height) * displayScale - Double(height)) <= 0.000_001 else {
            throw HelperFailure.runtime("unsupported diagnostic content transform")
        }
    }

    func sample(_ buffer: CVPixelBuffer) throws -> [UInt8] {
        guard let space = CVImageBufferGetColorSpace(buffer), space.takeUnretainedValue().name == CGColorSpace.sRGB,
              CVPixelBufferGetPixelFormatType(buffer) == kCVPixelFormatType_32BGRA,
              CVPixelBufferGetWidth(buffer) == width, CVPixelBufferGetHeight(buffer) == height,
              CVPixelBufferLockBaseAddress(buffer, .readOnly) == kCVReturnSuccess else {
            throw HelperFailure.runtime("diagnostic scene buffer format or geometry changed")
        }
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(buffer) else {
            throw HelperFailure.runtime("diagnostic scene buffer has no pixels")
        }
        let stride = CVPixelBufferGetBytesPerRow(buffer)
        var samples: [UInt8] = []
        samples.reserveCapacity(135)
        for (x, y) in points {
            let offset = y * stride + x * 4
            guard base.load(fromByteOffset: offset + 3, as: UInt8.self) == 255 else {
                throw HelperFailure.runtime("diagnostic scene sample is not opaque")
            }
            for channel in [2, 1, 0] {
                samples.append(base.load(fromByteOffset: offset + channel, as: UInt8.self))
            }
        }
        return samples
    }
}

func sceneSelfTest() throws {
    func colors(_ values: [UInt8]) -> [UInt8] {
        values.flatMap { Array(repeating: $0, count: 27) }
    }
    try check(sceneState(colors(sceneExpectedA)) == "A", "scene A")
    try check(sceneState(colors(sceneExpectedB)) == "B", "scene B")
    for bad in [[0, 181, 223, 241, 255], [0, 188, 118, 225, 255], [0, 0, 0, 0, 0]] {
        try check(sceneState(colors(bad.map(UInt8.init))) == "invalid", "invalid scene")
    }
    var edge = colors(sceneExpectedA)
    edge[27] += UInt8(sceneTolerance)
    try check(sceneState(edge) == "A", "inclusive scene tolerance")
    edge[27] += 1
    try check(sceneState(edge) == "invalid", "scene tolerance overflow")
    edge = colors(sceneExpectedA)
    edge[28] = 200
    try check(sceneState(edge) == "invalid", "non-gray scene")
    try check(sceneState(Array(edge.dropLast())) == "invalid", "missing scene sample")

    let region = CGRect(x: 0, y: 0, width: 100, height: 40)
    let sampler = try SceneSampler(width: 100, height: 40, region: region, scale: 1)
    try sampler.validateTransform(rect: region, contentScale: 1, displayScale: 1, requestedScale: 1)
    do {
        try sampler.validateTransform(rect: region, contentScale: 1, displayScale: 2, requestedScale: 1)
        throw HelperFailure.usage("wrong transform accepted")
    } catch HelperFailure.runtime { }
    var created: CVPixelBuffer?
    let status = CVPixelBufferCreate(nil, 100, 40, kCVPixelFormatType_32BGRA, nil, &created)
    guard status == kCVReturnSuccess, let buffer = created else {
        throw HelperFailure.runtime("cannot create diagnostic scene test buffer")
    }
    guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else {
        throw HelperFailure.runtime("cannot create test sRGB color space")
    }
    CVBufferSetAttachment(buffer, kCVImageBufferCGColorSpaceKey, colorSpace, .shouldPropagate)
    CVPixelBufferLockBaseAddress(buffer, [])
    guard let base = CVPixelBufferGetBaseAddress(buffer) else {
        CVPixelBufferUnlockBaseAddress(buffer, [])
        throw HelperFailure.runtime("test buffer has no pixels")
    }
    let stride = CVPixelBufferGetBytesPerRow(buffer)
    for y in 0..<40 {
        for x in 0..<100 {
            for channel in 0..<4 {
                base.storeBytes(of: channel == 3 ? UInt8(255) : sceneExpectedA[x / 20],
                                toByteOffset: y * stride + x * 4 + channel, as: UInt8.self)
            }
        }
    }
    CVPixelBufferUnlockBaseAddress(buffer, [])
    try check(try sampler.sample(buffer) == colors(sceneExpectedA), "raw RGB sampling")
    let cropped = try SceneSampler(width: 100, height: 40,
                                  region: CGRect(x: 0, y: 0, width: 50, height: 40), scale: 1)
    try check(sceneState(try cropped.sample(buffer)) == "invalid", "cropped pattern rejected")
    CVPixelBufferLockBaseAddress(buffer, [])
    if let pixels = CVPixelBufferGetBaseAddress(buffer), let (x, y) = sampler.points.first {
        pixels.storeBytes(of: UInt8(0), toByteOffset: y * stride + x * 4 + 3, as: UInt8.self)
    }
    CVPixelBufferUnlockBaseAddress(buffer, [])
    do {
        _ = try sampler.sample(buffer)
        throw HelperFailure.usage("transparent scene sample accepted")
    } catch HelperFailure.runtime { }

    for scale: CGFloat in [0, .nan, .infinity, 5] {
        do {
            _ = try SceneSampler(width: 100, height: 40, region: region, scale: scale)
            throw HelperFailure.runtime("invalid scene scale accepted")
        } catch HelperFailure.usage { }
    }
    for bad in [CGRect(x: -1, y: 0, width: 100, height: 40),
                CGRect(x: 0, y: 0, width: 101, height: 40),
                CGRect(x: 0, y: 0, width: 49, height: 40),
                CGRect(x: 0.5, y: 0, width: 50, height: 40)] {
        do {
            _ = try SceneSampler(width: 100, height: 40, region: bad, scale: 1)
            throw HelperFailure.runtime("invalid scene region accepted")
        } catch HelperFailure.usage { }
    }
    let wrong = try SceneSampler(width: 200, height: 80, region: region, scale: 2)
    do {
        _ = try wrong.sample(buffer)
        throw HelperFailure.usage("wrong-size scene buffer accepted")
    } catch HelperFailure.runtime { }
}
