import XCTest

final class EditorSmoke: XCTestCase {
    @MainActor
    func testEditor() throws {
        let app = XCUIApplication()
        app.launchEnvironment["FDL_NOBLINK"] = "1"
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-prototype-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launchEnvironment["QUIET_MD_IOS_VERIFY_BACKGROUND"] = "1"
        app.launch()
        XCTAssertEqual(app.state, .runningForeground)
        let origin = app.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
        func at(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
            origin.withOffset(CGVector(dx: x, dy: y))
        }
        func capture(_ name: String) {
            let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            shot.name = name
            shot.lifetime = .keepAlways
            add(shot)
        }
        // 第一段在顶栏之下：y 248。「长按拖动选择中文」在 x 224…352，「Quiet」在 x 168…205。
        at(290, 248).press(forDuration: 0.75, thenDragTo: at(340, 248))
        capture("Touch selection")
        at(187, 248).doubleTap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        capture("Double tap")
        for (x, y) in [(280.0, 720.0), (300, 612), (239, 668), (43, 668), (340, 612)] {
            at(x, y).tap()
        }
        capture("Pinyin candidates")
        let candidate = app.keyboards.firstMatch.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "你好")).firstMatch
        if candidate.exists { candidate.tap() } else { at(47, 561).tap() }
        capture("Committed Chinese")
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5) || app.state == .runningBackgroundSuspended)
    }
}
