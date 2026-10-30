import Testing

@Suite struct LaunchTests {
  @Test func launchOpensTheMainWindow() async throws {
    let app = try LaunchedApp.start()
    defer { app.terminate() }

    try await Driver.waitForWindow(titled: "Bilbo", in: app, timeout: .seconds(15))
  }
}
