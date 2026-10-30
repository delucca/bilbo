import Foundation

struct DriverError: Error, CustomStringConvertible {
  let description: String
}

final class LaunchedApp {
  let process: Process

  private init(process: Process) {
    self.process = process
  }

  static func start() throws -> LaunchedApp {
    let bundle = try unpackApp()
    let process = Process()
    process.executableURL = bundle.appending(path: "Contents/MacOS/Bilbo")
    try process.run()
    return LaunchedApp(process: process)
  }

  var pid: Int32 { process.processIdentifier }

  func terminate() {
    guard process.isRunning else { return }
    process.terminate()
    process.waitUntilExit()
  }

  // rules_apple outputs the .app zipped; each launch gets its own copy in the test's temp dir.
  private static func unpackApp() throws -> URL {
    let destination = try temporaryDirectory().appending(path: UUID().uuidString)
    try run("/usr/bin/ditto", ["-x", "-k", try runfile("BILBO_APP").path, destination.path]).check()
    return destination.appending(path: "Bilbo.app")
  }
}

enum Driver {
  static func waitForWindow(titled title: String, in app: LaunchedApp, timeout: Duration) async throws {
    let permissions = try axorc(["permissions"])
    guard permissions.status == 0, permissions.output.contains(": granted") else {
      throw DriverError(description: "axorc lacks the Accessibility permission: \(permissions.output)\(permissions.errors)")
    }

    let deadline = ContinuousClock.now + timeout
    var lastError = ""
    repeat {
      guard app.process.isRunning else {
        throw DriverError(description: "Bilbo exited with status \(app.process.terminationStatus) before showing a window")
      }
      let find = try axorc(["find", "--app", "\(app.pid)", "--role", "AXWindow", "--title", title, "--depth", "2"])
      if find.status == 0 { return }
      lastError = find.errors
      try await Task.sleep(for: .milliseconds(250))
    } while ContinuousClock.now < deadline

    throw DriverError(description: "no window titled '\(title)' after \(timeout); last axorc error: \(lastError)")
  }

  private static func axorc(_ arguments: [String]) throws -> RunResult {
    try run(try runfile("AXORC").path, arguments)
  }
}

struct RunResult {
  let executable: String
  let status: Int32
  let output: String
  let errors: String

  func check() throws {
    guard status == 0 else {
      throw DriverError(description: "\(executable) exited with \(status): \(errors)")
    }
  }
}

private func runfile(_ variable: String) throws -> URL {
  let environment = ProcessInfo.processInfo.environment
  guard let root = environment["TEST_SRCDIR"], let path = environment[variable] else {
    throw DriverError(description: "\(variable) or TEST_SRCDIR is unset; run this test through bazel test")
  }
  return URL(filePath: root).appending(path: path)
}

private func temporaryDirectory() throws -> URL {
  guard let path = ProcessInfo.processInfo.environment["TEST_TMPDIR"] else {
    throw DriverError(description: "TEST_TMPDIR is unset; run this test through bazel test")
  }
  return URL(filePath: path)
}

// A hung child must not outlive the test's own timeout: Bazel then kills only the test's
// process group, skipping the cleanup that quits Bilbo.
private func run(_ executable: String, _ arguments: [String], limit: Duration = .seconds(5)) throws -> RunResult {
  let process = Process()
  process.executableURL = URL(filePath: executable)
  process.arguments = arguments
  let output = Pipe()
  let errors = Pipe()
  process.standardOutput = output
  process.standardError = errors
  try process.run()

  let deadline = ContinuousClock.now + limit
  while process.isRunning && ContinuousClock.now < deadline {
    Thread.sleep(forTimeInterval: 0.05)
  }
  if process.isRunning {
    process.terminate()
    process.waitUntilExit()
    throw DriverError(description: "\(executable) \(arguments.joined(separator: " ")) did not finish within \(limit)")
  }

  return RunResult(
    executable: executable,
    status: process.terminationStatus,
    output: String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self),
    errors: String(decoding: errors.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
  )
}
