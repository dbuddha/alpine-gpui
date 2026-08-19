import AppKit
import CoreGraphics
import CoreVideo
import CryptoKit
import Darwin
import Foundation
import ImageIO
import ScreenCaptureKit
import UniformTypeIdentifiers

enum CaptureFailure: Error, CustomStringConvertible {
    case message(String)

    var description: String {
        switch self {
        case .message(let message): return message
        }
    }
}

struct Options {
    var values: [String: String]

    init(arguments: [String]) throws {
        if arguments == ["--self-test"] {
            values = ["self-test": "true"]
            return
        }
        guard arguments.count.isMultiple(of: 2) else {
            throw CaptureFailure.message("arguments must be --name value pairs")
        }
        var parsed: [String: String] = [:]
        for index in stride(from: 0, to: arguments.count, by: 2) {
            let key = arguments[index]
            guard key.hasPrefix("--"), parsed[key] == nil else {
                throw CaptureFailure.message("invalid or duplicate argument \(key)")
            }
            parsed[String(key.dropFirst(2))] = arguments[index + 1]
        }
        values = parsed
    }

    func require(_ name: String) throws -> String {
        guard let value = values[name], !value.isEmpty else {
            throw CaptureFailure.message("missing --\(name)")
        }
        return value
    }
}

struct PatchResult {
    let samples: [UInt8]
    let acceptedError: UInt8
    let controlError: UInt8
    let qualified: Bool
}

let acceptedExpected: [UInt8] = [0, 118, 188, 225, 255]
let wrongExpected: [UInt8] = [0, 181, 223, 241, 255]

func maxError(_ samples: [UInt8], _ expected: [UInt8]) -> UInt8 {
    zip(samples, expected).map { UInt8(abs(Int($0) - Int($1))) }.max() ?? .max
}

func classify(samples: [UInt8], control: String) throws -> PatchResult {
    guard samples.count == acceptedExpected.count else {
        throw CaptureFailure.message("exactly five patch samples are required")
    }
    let acceptedError = maxError(samples, acceptedExpected)
    let controlError = maxError(samples, wrongExpected)
    let qualified: Bool
    if control == "accepted" {
        qualified = acceptedError <= 12
    } else if control == "wrong-transfer" {
        qualified = acceptedError >= 30 && controlError <= 12
    } else {
        throw CaptureFailure.message("unsupported control \(control)")
    }
    return PatchResult(
        samples: samples,
        acceptedError: acceptedError,
        controlError: controlError,
        qualified: qualified
    )
}

func sha256(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

func sysctlString(_ name: String) -> String {
    var size = 0
    guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 1 else { return "unknown" }
    var bytes = [UInt8](repeating: 0, count: size)
    guard sysctlbyname(name, &bytes, &size, nil, 0) == 0 else { return "unknown" }
    return String(cString: bytes)
}

func activeDisplays() throws -> [CGDirectDisplayID] {
    var count: UInt32 = 0
    guard CGGetActiveDisplayList(0, nil, &count) == .success, count > 0 else {
        throw CaptureFailure.message("cannot enumerate active displays")
    }
    var displays = [CGDirectDisplayID](repeating: 0, count: Int(count))
    guard CGGetActiveDisplayList(count, &displays, &count) == .success else {
        throw CaptureFailure.message("cannot read active display identities")
    }
    return Array(displays.prefix(Int(count)))
}

func displayForWindow(_ frame: CGRect, displays: [CGDirectDisplayID]) throws -> CGDirectDisplayID {
    let ranked = displays.map { display in
        (display, frame.intersection(CGDisplayBounds(display)).width * frame.intersection(CGDisplayBounds(display)).height)
    }
    guard let selected = ranked.max(by: { $0.1 < $1.1 }), selected.1 > 0 else {
        throw CaptureFailure.message("captured window does not intersect an active display")
    }
    return selected.0
}

func findWindow(pid: pid_t, title: String) async throws -> SCWindow {
    for _ in 0..<50 {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        if let window = content.windows.first(where: {
            $0.owningApplication?.processID == pid && $0.title == title && $0.isOnScreen
        }) {
            return window
        }
        try await Task.sleep(for: .milliseconds(100))
    }
    throw CaptureFailure.message("onscreen Alpine window was not discoverable within five seconds")
}

func rgbaBytes(_ image: CGImage) throws -> [UInt8] {
    let width = image.width
    let height = image.height
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    let created = bytes.withUnsafeMutableBytes { storage -> Bool in
        guard let base = storage.baseAddress,
              let space = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(
                data: base,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: space,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                    | CGBitmapInfo.byteOrder32Big.rawValue
              ) else { return false }
        context.interpolationQuality = .none
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return true
    }
    guard created else { throw CaptureFailure.message("cannot normalize capture into sRGB RGBA") }
    return bytes
}

func samplePatches(_ image: CGImage) throws -> [UInt8] {
    let bytes = try rgbaBytes(image)
    let width = image.width
    let height = image.height
    let y = min(height - 1, max(0, Int(Double(height) * 0.70)))
    return (0..<5).map { patch in
        let center = (Double(patch) + 0.5) / 5.0
        let x = min(width - 1, max(0, Int(Double(width) * center)))
        var total = 0
        var count = 0
        for row in max(0, y - 4)...min(height - 1, y + 4) {
            for column in max(0, x - 4)...min(width - 1, x + 4) {
                let offset = (row * width + column) * 4
                total += Int(bytes[offset]) + Int(bytes[offset + 1]) + Int(bytes[offset + 2])
                count += 3
            }
        }
        return UInt8(total / count)
    }
}

func tomlString(_ value: String) -> String {
    let escaped = value
        .replacingOccurrences(of: "\\", with: "\\\\")
        .replacingOccurrences(of: "\"", with: "\\\"")
        .replacingOccurrences(of: "\n", with: "\\n")
        .replacingOccurrences(of: "\r", with: "\\r")
    return "\"\(escaped)\""
}

func writePNG(_ image: CGImage, to url: URL) throws {
    guard let destination = CGImageDestinationCreateWithURL(
        url as CFURL,
        UTType.png.identifier as CFString,
        1,
        nil
    ) else { throw CaptureFailure.message("cannot create PNG destination") }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        throw CaptureFailure.message("cannot finalize PNG capture")
    }
}

@main
struct CaptureMain {
    static func main() async {
        do {
            let options = try Options(arguments: Array(CommandLine.arguments.dropFirst()))
            if options.values["self-test"] == "true" {
                try selfTest()
                print("onscreen SDR capture self-test passed")
                return
            }
            try await capture(options)
        } catch {
            FileHandle.standardError.write(Data("onscreen SDR capture error: \(error)\n".utf8))
            exit(1)
        }
    }

    static func selfTest() throws {
        let accepted = try classify(samples: acceptedExpected, control: "accepted")
        let wrong = try classify(samples: wrongExpected, control: "wrong-transfer")
        guard accepted.qualified, wrong.qualified,
              !(try classify(samples: wrongExpected, control: "accepted")).qualified,
              !(try classify(samples: acceptedExpected, control: "wrong-transfer")).qualified else {
            throw CaptureFailure.message("transfer comparator did not discriminate controls")
        }
    }

    static func capture(_ options: Options) async throws {
        guard CGPreflightScreenCaptureAccess() else {
            throw CaptureFailure.message("Screen Recording permission is required before qualification")
        }
        guard let pid = pid_t(try options.require("pid")),
              let sceneRevision = UInt64(try options.require("scene-revision")),
              let logicalWidth = Double(try options.require("logical-width")),
              let logicalHeight = Double(try options.require("logical-height")),
              let expectedScale = Double(try options.require("backing-scale")),
              let presentedTimeBits = UInt64(try options.require("presented-time-bits")) else {
            throw CaptureFailure.message("numeric driver arguments are invalid")
        }
        let title = try options.require("title")
        let stage = try options.require("stage")
        let control = try options.require("control")
        let revision = try options.require("revision")
        let sceneURL = URL(fileURLWithPath: try options.require("scene"))
        let output = URL(fileURLWithPath: try options.require("output"), isDirectory: true)
        guard pid > 0, revision.count == 40, sceneRevision > 0,
              logicalWidth > 0, logicalHeight > 0, expectedScale > 0,
              presentedTimeBits != 0 else {
            throw CaptureFailure.message("invalid driver identity or geometry")
        }

        let window = try await findWindow(pid: pid, title: title)
        let displays = try activeDisplays()
        let display = try displayForWindow(window.frame, displays: displays)
        let bounds = CGDisplayBounds(display)
        let scale = Double(CGDisplayPixelsWide(display)) / bounds.width
        guard abs(scale - expectedScale) <= 0.01 else {
            throw CaptureFailure.message("capture display scale \(scale) differs from AppKit scale \(expectedScale)")
        }

        let configuration = SCStreamConfiguration()
        configuration.width = Int(window.frame.width * scale)
        configuration.height = Int(window.frame.height * scale)
        configuration.pixelFormat = kCVPixelFormatType_32BGRA
        configuration.colorSpaceName = CGColorSpace.sRGB as CFString
        configuration.showsCursor = false
        configuration.shouldBeOpaque = true
        configuration.ignoreShadowsSingleWindow = true
        let filter = SCContentFilter(desktopIndependentWindow: window)
        let image = try await SCScreenshotManager.captureImage(
            contentFilter: filter,
            configuration: configuration
        )
        let samples = try samplePatches(image)
        let result = try classify(samples: samples, control: control)
        guard result.qualified else {
            throw CaptureFailure.message(
                "patch comparator rejected \(stage): samples=\(samples), accepted-error=\(result.acceptedError), control-error=\(result.controlError)"
            )
        }

        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let captureName = "\(stage).png"
        let profileName = "\(stage)-display.icc"
        let reportName = "\(stage).toml"
        let captureURL = output.appendingPathComponent(captureName)
        let profileURL = output.appendingPathComponent(profileName)
        let reportURL = output.appendingPathComponent(reportName)
        try writePNG(image, to: captureURL)

        let displaySpace = CGDisplayCopyColorSpace(display)
        guard let profileData = displaySpace.copyICCData() else {
            throw CaptureFailure.message("active display has no readable ICC profile")
        }
        let profile = profileData as Data
        try profile.write(to: profileURL, options: Data.WritingOptions.atomic)
        let sceneData = try Data(contentsOf: sceneURL)
        let captureData = try Data(contentsOf: captureURL)
        let captureSpace = image.colorSpace?.name as String? ?? "unknown"
        let report = """
        schema = \(tomlString("alpine-onscreen-sdr-capture/v1"))
        task_issue = 234
        revision = \(tomlString(revision))
        stage = \(tomlString(stage))
        control = \(tomlString(control))
        os_build = \(tomlString(sysctlString("kern.osversion")))
        hardware_model = \(tomlString(sysctlString("hw.model")))
        screen_capture_permission = true
        display_count = \(displays.count)
        window_id = \(window.windowID)
        display_id = \(display)
        backing_scale = \(scale)
        logical_width = \(logicalWidth)
        logical_height = \(logicalHeight)
        capture_width = \(image.width)
        capture_height = \(image.height)
        target_format = "BGRA8Unorm_sRGB"
        layer_color_space = "kCGColorSpaceSRGB"
        capture_color_space = "kCGColorSpaceSRGB"
        capture_image_color_space = \(tomlString(captureSpace))
        display_profile_name = \(tomlString(displaySpace.name as String? ?? "unnamed"))
        extended_dynamic_range = false
        scene_revision = \(sceneRevision)
        presented_time_bits = \(presentedTimeBits)
        scene_file = \(tomlString(sceneURL.lastPathComponent))
        scene_sha256 = \(tomlString(sha256(sceneData)))
        capture_file = \(tomlString(captureName))
        capture_sha256 = \(tomlString(sha256(captureData)))
        display_profile_file = \(tomlString(profileName))
        display_profile_sha256 = \(tomlString(sha256(profile)))
        samples = \(samples)
        accepted_expected = \(acceptedExpected)
        control_expected = \(wrongExpected)
        accepted_max_error = \(result.acceptedError)
        control_max_error = \(result.controlError)
        qualified = true
        performance_claim = false
        """ + "\n"
        try Data(report.utf8).write(to: reportURL, options: .atomic)
        print("captured \(stage) window=\(window.windowID) display=\(display) samples=\(samples)")
    }
}
