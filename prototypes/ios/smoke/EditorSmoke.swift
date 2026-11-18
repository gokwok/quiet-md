import XCTest

final class EditorSmoke: XCTestCase {
    @MainActor
    func testSelectionOpensLinkPanel() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires Liquid Glass") }
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-glass-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let word = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 187, dy: 248))
        word.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        word.doubleTap()
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "Selection toolbar"; shot.lifetime = .keepAlways; add(shot)
        // GPUI 工具栏的链接按钮，位于键盘上沿上方。
        let origin = app.coordinate(withNormalizedOffset: .zero)
        origin.withOffset(CGVector(dx: 276, dy: 516)).tap()
        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("https://example.org/quiet")
        let confirm = app.buttons["quiet.link.checkmark"]
        XCTAssertTrue(confirm.isEnabled)
        confirm.tap()
        XCTAssertTrue(field.waitForNonExistence(timeout: 5))
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5) || app.state == .runningBackgroundSuspended)
    }

    @MainActor
    func testDocumentPanelSearch() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires native sheets") }
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-find-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let origin = app.coordinate(withNormalizedOffset:.zero)
        origin.withOffset(CGVector(dx:310,dy:88)).tap()
        let tabs = app.segmentedControls.firstMatch
        XCTAssertTrue(tabs.waitForExistence(timeout:5))
        func capture(_ name:String) {
            let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot())
            shot.name=name; shot.lifetime = .keepAlways; add(shot)
        }
        capture("Document outline")
        tabs.buttons.element(boundBy:3).tap()
        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout:5))
        field.tap(); field.typeText("Quiet")
        app.buttons["quiet.find.chevron.down"].tap()
        app.buttons.matching(NSPredicate(format:"label IN %@",["查找和替换","Find and replace","Find and Replace"])).firstMatch.tap()
        let replacement = app.textFields.element(boundBy:1)
        replacement.tap(); replacement.typeText("Notes")
        capture("Find and replace")
        app.buttons["quiet.find.slider.horizontal.3"].tap()
        capture("Find options")
    }

    @MainActor
    func testTableSheetInsertsRow() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires native sheets") }
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-table-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let origin=app.coordinate(withNormalizedOffset:.zero)
        origin.withOffset(CGVector(dx:50,dy:412)).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout:5))
        origin.withOffset(CGVector(dx:125,dy:516)).tap()
        app.buttons.matching(NSPredicate(format:"label IN %@",["选择单元格","Select Cells"])).firstMatch.tap()
        let edit = app.buttons["quiet.selection.tablecells"]
        XCTAssertTrue(edit.waitForExistence(timeout:5))
        func capture(_ name:String) {
            let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot())
            shot.name=name; shot.lifetime = .keepAlways; add(shot)
        }
        capture("Table tools")
        app.buttons["quiet.selection.textformat"].tap()
        capture("Cell formatting")
        app.buttons["quiet.close.selection"].tap()
        edit.tap(); capture("Table structure")
        app.buttons.matching(NSPredicate(format:"label IN %@",["插入行","Insert row"])).firstMatch.tap()
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for:.runningBackground,timeout:5) || app.state == .runningBackgroundSuspended)
    }

    @MainActor
    func testSourceEditors() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires native sheets") }
        let app = XCUIApplication()
        app.launchEnvironment["FDL_NOBLINK"] = "1"
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-source-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let origin = app.coordinate(withNormalizedOffset: .zero)
        func at(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate { origin.withOffset(CGVector(dx:x,dy:y)) }
        func capture(_ name:String) {
            let shot = XCTAttachment(screenshot:XCUIScreen.main.screenshot())
            shot.name=name; shot.lifetime = .keepAlways; add(shot)
        }
        at(128,505).tap()
        let formula = app.textViews["LaTeX"]
        XCTAssertTrue(formula.waitForExistence(timeout:5))
        formula.tap()
        formula.typeText("+1")
        capture("Equation editor")
        app.buttons["quiet.close.equation"].tap()
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for:.runningBackground,timeout:5) || app.state == .runningBackgroundSuspended)
        app.terminate()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-code-check-\(UUID().uuidString)"
        app.launch()
        at(145,700).tap()
        let code = app.textViews["quiet.field.language"]
        XCTAssertTrue(code.waitForExistence(timeout:5))
        capture("Code editor")
        app.buttons["quiet.language.chevron.up.chevron.down"].tap()
        capture("Language menu")
        app.buttons["Bash"].tap()
        code.tap(); code.typeText(" + 7")
        app.buttons["quiet.close.language"].tap()
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for:.runningBackground,timeout:5) || app.state == .runningBackgroundSuspended)
    }

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
        at(187, 248).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
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
