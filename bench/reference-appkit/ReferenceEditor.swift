import AppKit

// A minimal native editor: one NSTextView with the editor's font metrics, no
// spelling, substitutions or completion. It is the native floor that Alpine
// and Zed are compared against, so it stays as plain as AppKit allows.

let fontName = "Menlo-Regular"
let fontSize: CGFloat = 15
let lineHeight: CGFloat = 22

struct Launch {
    var path: String?
    var width: CGFloat = 960
    var height: CGFloat = 540
    var selfTest = false

    init(_ arguments: [String]) throws {
        var index = 0
        while index < arguments.count {
            let argument = arguments[index]
            switch argument {
            case "--self-test":
                selfTest = true
                index += 1
            case "--width", "--height":
                guard index + 1 < arguments.count, let value = Double(arguments[index + 1]),
                      value >= 200, value <= 8_000 else {
                    throw ReferenceFailure("\(argument) needs a number from 200 to 8000")
                }
                if argument == "--width" { width = value } else { height = value }
                index += 2
            case let flag where flag.hasPrefix("-") && !flag.hasPrefix("--"):
                // `-Key value` pairs belong to NSUserDefaults' argument domain,
                // such as -ApplePersistenceIgnoreState YES; AppKit reads them.
                guard index + 1 < arguments.count else {
                    throw ReferenceFailure("\(flag) needs a value")
                }
                index += 2
            default:
                guard path == nil, !argument.hasPrefix("-") else {
                    throw ReferenceFailure(
                        "usage: bench-reference-appkit [-Key value] [--width W] [--height H] [FILE]"
                    )
                }
                path = argument
                index += 1
            }
        }
    }
}

struct ReferenceFailure: Error, CustomStringConvertible {
    let description: String

    init(_ description: String) {
        self.description = description
    }
}

func editorAttributes() -> [NSAttributedString.Key: Any] {
    let paragraph = NSMutableParagraphStyle()
    paragraph.minimumLineHeight = lineHeight
    paragraph.maximumLineHeight = lineHeight
    let font = NSFont(name: fontName, size: fontSize)
        ?? NSFont.monospacedSystemFont(ofSize: fontSize, weight: .regular)
    return [.font: font, .paragraphStyle: paragraph, .foregroundColor: NSColor.textColor]
}

func makeEditor(text: String) -> NSScrollView? {
    let scroll = NSTextView.scrollableTextView()
    guard let view = scroll.documentView as? NSTextView else { return nil }
    view.isRichText = false
    view.importsGraphics = false
    view.allowsUndo = true
    view.usesFindBar = true
    view.isContinuousSpellCheckingEnabled = false
    view.isGrammarCheckingEnabled = false
    view.isAutomaticSpellingCorrectionEnabled = false
    view.isAutomaticQuoteSubstitutionEnabled = false
    view.isAutomaticDashSubstitutionEnabled = false
    view.isAutomaticTextReplacementEnabled = false
    view.isAutomaticLinkDetectionEnabled = false
    view.isAutomaticDataDetectionEnabled = false
    view.isAutomaticTextCompletionEnabled = false
    view.smartInsertDeleteEnabled = false
    let attributes = editorAttributes()
    view.typingAttributes = attributes
    view.textStorage?.setAttributedString(NSAttributedString(string: text, attributes: attributes))
    view.setSelectedRange(NSRange(location: 0, length: 0))
    return scroll
}

func installMenu(appName: String) {
    let main = NSMenu()
    let appItem = NSMenuItem()
    let appMenu = NSMenu()
    appMenu.addItem(withTitle: "Quit \(appName)", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
    appItem.submenu = appMenu
    main.addItem(appItem)
    let editItem = NSMenuItem()
    let editMenu = NSMenu(title: "Edit")
    editMenu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
    editMenu.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
    editMenu.addItem(.separator())
    editMenu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
    editMenu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
    editMenu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
    editMenu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
    editItem.submenu = editMenu
    main.addItem(editItem)
    NSApp.mainMenu = main
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private let launch: Launch
    private let text: String
    private var window: NSWindow?

    init(launch: Launch, text: String) {
        self.launch = launch
        self.text = text
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: launch.width, height: launch.height),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.isRestorable = false
        window.title = launch.path.map { URL(fileURLWithPath: $0).lastPathComponent } ?? "Untitled"
        guard let editor = makeEditor(text: text) else {
            FileHandle.standardError.write(Data("error: NSTextView setup failed\n".utf8))
            exit(1)
        }
        window.contentView = editor
        // The first trial measured a 944x562 frame, so the content size is
        // set again after the scroll view is installed.
        window.setContentSize(NSSize(width: launch.width, height: launch.height))
        window.center()
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(editor.documentView)
        self.window = window
        let frame = window.frame
        FileHandle.standardError.write(Data(
            "window frame \(Int(frame.width))x\(Int(frame.height))\n".utf8
        ))
        // The call Alpine and Zed make. On macOS 26.6 a child of the terminal
        // comes forward with it; the cooperative activate() did not in the
        // first bench run, which then refused to measure.
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

func selfTest() throws {
    guard let editor = makeEditor(text: "alpha\nbeta\n"),
          let view = editor.documentView as? NSTextView else {
        throw ReferenceFailure("self-test failed: editor setup")
    }
    guard view.string == "alpha\nbeta\n", !view.isContinuousSpellCheckingEnabled,
          view.selectedRange().location == 0 else {
        throw ReferenceFailure("self-test failed: editor state")
    }
    let font = view.textStorage?.attribute(.font, at: 0, effectiveRange: nil) as? NSFont
    guard font?.pointSize == fontSize else {
        throw ReferenceFailure("self-test failed: font size")
    }
    let launch = try Launch([
        "-ApplePersistenceIgnoreState", "YES", "--width", "960", "--height", "540", "/f.txt",
    ])
    guard launch.path == "/f.txt", launch.width == 960, launch.height == 540 else {
        throw ReferenceFailure("self-test failed: AppKit argument pairs")
    }
    guard (try? Launch(["/a.txt", "/b.txt"])) == nil, (try? Launch(["-Dangling"])) == nil else {
        throw ReferenceFailure("self-test failed: bad arguments accepted")
    }
    print("self-test\tok")
}

@main
struct ReferenceEditorMain {
    static func main() {
        do {
            let launch = try Launch(Array(CommandLine.arguments.dropFirst()))
            if launch.selfTest {
                try selfTest()
                return
            }
            let text = try launch.path.map { try String(contentsOfFile: $0, encoding: .utf8) } ?? ""
            // Registration defaults live in memory only. They stop the caret
            // blink and window restoration without writing preferences.
            UserDefaults.standard.register(defaults: [
                "NSTextInsertionPointBlinkPeriodOn": 1.0e9,
                "NSTextInsertionPointBlinkPeriodOff": 0.0,
                "ApplePersistenceIgnoreState": true,
            ])
            let app = NSApplication.shared
            app.setActivationPolicy(.regular)
            installMenu(appName: "Bench Reference")
            let delegate = AppDelegate(launch: launch, text: text)
            app.delegate = delegate
            app.run()
        } catch {
            FileHandle.standardError.write(Data("error: \(error)\n".utf8))
            exit(1)
        }
    }
}
