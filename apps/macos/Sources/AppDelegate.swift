import AppKit

@main
final class AppDelegate: NSObject, NSApplicationDelegate {
  private var window: NSWindow?

  static func main() {
    let app = NSApplication.shared
    let delegate = AppDelegate()
    app.delegate = delegate
    app.run()
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    NSApp.mainMenu = makeMainMenu()

    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1440, height: 900),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.title = "Bilbo"
    window.center()
    window.makeKeyAndOrderFront(nil)
    self.window = window

    NSApp.activate()
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    true
  }

  private func makeMainMenu() -> NSMenu {
    let appMenu = NSMenu()
    appMenu.addItem(
      withTitle: "Quit Bilbo",
      action: #selector(NSApplication.terminate(_:)),
      keyEquivalent: "q"
    )

    let appItem = NSMenuItem()
    appItem.submenu = appMenu

    let mainMenu = NSMenu()
    mainMenu.addItem(appItem)
    return mainMenu
  }
}
