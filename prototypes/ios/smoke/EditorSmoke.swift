import XCTest

final class EditorSmoke: XCTestCase {
    @MainActor
    func testTextSelectionHandlesKeepKeyboard() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires phone selection handles") }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-range-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let origin = app.coordinate(withNormalizedOffset: .zero)
        func at(_ x: CGFloat, _ y: CGFloat) -> XCUICoordinate {
            origin.withOffset(CGVector(dx: x, dy: y))
        }
        let word = at(187, 248)
        let keyboard = app.keyboards.firstMatch
        word.tap()
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        word.doubleTap()
        // 「Quiet」的两个手柄。向左拉到段首，再向下跨段拉到第二段末尾。
        at(171, 234).press(forDuration: 0.05, thenDragTo: at(16, 234))
        at(210, 268).press(forDuration: 0.05, thenDragTo: at(210, 335))
        XCTAssertTrue(keyboard.exists, "Dragging a text selection should keep the keyboard visible")
        app.typeText("x")
        // 普通正文下滑依然能收键盘，再点一下正文应该重新开始输入。
        at(160, 248).press(forDuration: 0.05, thenDragTo: at(160, 425), withVelocity: .slow, thenHoldForDuration: 0.5)
        XCTAssertTrue(keyboard.waitForNonExistence(timeout: 5))
        at(26, 248).tap()
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5) || app.state == .runningBackgroundSuspended)
    }

    @MainActor
    func testKeyboardDeletesEmptyParagraph() throws {
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-backspace-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let origin = app.coordinate(withNormalizedOffset: .zero)
        // 第一段末尾回车，再点软键盘的退格；两个数字应连在同一段。
        origin.withOffset(CGVector(dx: 220, dy: 284)).tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        app.typeText("1\n")
        let delete = keyboard.keys["delete"]
        XCTAssertTrue(delete.waitForExistence(timeout: 5))
        delete.tap()
        app.typeText("2")
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5) || app.state == .runningBackgroundSuspended)
    }

    @MainActor
    func testFormatPanelConsumesOutsideTouch() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires native format panel") }
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-format-touch-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let origin = app.coordinate(withNormalizedOffset: .zero)
        let word = origin.withOffset(CGVector(dx: 187, dy: 248))
        word.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        origin.withOffset(CGVector(dx: 78, dy: 516)).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        // 静止后抬指，避免 UIKit 把下一次轻点用来停止横向惯性。
        word.press(forDuration: 0.05, thenDragTo: origin.withOffset(CGVector(dx: 290, dy: 248)),
                   withVelocity: .slow, thenHoldForDuration: 0.5)
        word.tap()
        XCTAssertFalse(app.keyboards.firstMatch.waitForExistence(timeout: 1))
        word.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
    }

    @MainActor
    func testSelectionOpensLinkPanel() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires Liquid Glass") }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-glass-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launchEnvironment["QUIET_MD_IOS_VERIFY_BACKGROUND"] = "1"
        app.launch()
        let word = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 187, dy: 248))
        let keyboard = app.keyboards.firstMatch
        word.tap()
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        word.doubleTap()
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = "Selection toolbar"; shot.lifetime = .keepAlways; add(shot)
        // GPUI 选区工具栏的第六个按钮。iPhone 18 Pro 的键盘 AX frame 不包含上方的候选条。
        let origin = app.coordinate(withNormalizedOffset: .zero)
        origin.withOffset(CGVector(dx: 276, dy: 516)).tap()
        let field = app.textFields.matching(NSPredicate(format: "label IN %@", ["链接", "Link"])).firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("https://example.org/quiet")
        let confirm = app.buttons["quiet.link.checkmark"]
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: confirm)
        wait(for: [enabled], timeout: 5)
        confirm.tap()
        XCTAssertTrue(field.waitForNonExistence(timeout: 5))
        XCUIDevice.shared.press(.home)
        let backgrounded = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            let state = app.state
            return state == .runningBackground || state == .runningBackgroundSuspended
        }, object: nil)
        wait(for: [backgrounded], timeout: 10)
    }

    @MainActor
    func testDocumentPanelSearch() throws {
        guard #available(iOS 26, *) else { throw XCTSkip("Requires native sheets") }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchEnvironment["QUIET_MD_HOME"] = "Quiet.md-find-check-\(UUID().uuidString)"
        app.launchEnvironment["QUIET_MD_IOS_SAMPLE"] = "1"
        app.launch()
        let catalogue = app.buttons["doc-catalogue"]
        XCTAssertTrue(catalogue.waitForExistence(timeout: 10))
        catalogue.tap()
        let tabs = app.segmentedControls.firstMatch
        XCTAssertTrue(tabs.waitForExistence(timeout:5))
        func capture(_ name:String) {
            let shot=XCTAttachment(screenshot:XCUIScreen.main.screenshot())
            shot.name=name; shot.lifetime = .keepAlways; add(shot)
        }
        capture("Document outline")
        tabs.buttons.element(boundBy:3).tap()
        let field = app.textFields.matching(NSPredicate(format: "label IN %@", ["在文档内查找", "Find in document"])).firstMatch
        XCTAssertTrue(field.waitForExistence(timeout:5))
        field.tap(); field.typeText("Quiet")
        let result = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "双击选择 Quiet")).firstMatch
        XCTAssertTrue(result.waitForExistence(timeout: 5))
        app.buttons["quiet.find.chevron.down"].tap()
        app.buttons.matching(NSPredicate(format:"label IN %@",["查找和替换","Find and replace","Find and Replace"])).firstMatch.tap()
        let replacement = app.textFields.matching(NSPredicate(format: "label IN %@", ["替换为", "Replace with"])).firstMatch
        XCTAssertTrue(replacement.waitForExistence(timeout: 5))
        replacement.tap(); replacement.typeText("Notes")
        capture("Find and replace")
        app.buttons["quiet.find.slider.horizontal.3"].tap()
        let regex = app.buttons.matching(NSPredicate(format: "label IN %@", ["正则表达式", "Regular expression"])).firstMatch
        XCTAssertTrue(regex.waitForExistence(timeout: 5))
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
