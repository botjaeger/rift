import ExpoModulesCore
import UIKit

enum CaptureMode {
  case terminal
  case shortcuts
}

/// Terminal mode: the view becomes first responder and takes every key (UIKeyInput for typed
/// text and the soft keyboard, pressesBegan for hardware keys, which repeat while held). Shortcuts
/// mode: the view wraps the command field and takes from it, as key commands, only ctrl and
/// ctrl+Option combinations, Escape, F-keys and, with captureTab, Tab / Shift+Tab. Option alone
/// composes a character on every iOS layout (å, ∫, @ …), which the field types. Cmd
/// combinations are left to the system in both.
class KeyCaptureView: ExpoView, UIKeyInput {
  let onText = EventDispatcher()
  let onKey = EventDispatcher()
  let onFocusChange = EventDispatcher()

  var mode: CaptureMode = .shortcuts {
    didSet {
      if mode != .terminal && isFirstResponder {
        _ = resignFirstResponder()
      }
    }
  }

  /// Shortcuts mode: Tab / Shift+Tab go to the pane (else they move the focus).
  var captureTab = false

  /// Presses whose began this view handled: their changed/ended/cancelled are not forwarded.
  private var handled = Set<UIPress>()

  /// The held named key that repeats (UIKit repeats only text input, not pressesBegan).
  private var repeatPress: UIPress?
  private var repeatTimer: Timer?
  static let repeatDelay: TimeInterval = 0.5
  static let repeatInterval: TimeInterval = 0.05

  // MARK: - UITextInputTraits: a terminal's keyboard, nothing rewritten

  var autocorrectionType: UITextAutocorrectionType = .no
  var autocapitalizationType: UITextAutocapitalizationType = .none
  var spellCheckingType: UITextSpellCheckingType = .no
  var smartQuotesType: UITextSmartQuotesType = .no
  var smartDashesType: UITextSmartDashesType = .no
  var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
  var keyboardType: UIKeyboardType = .asciiCapable
  var keyboardAppearance: UIKeyboardAppearance = .dark
  var returnKeyType: UIReturnKeyType = .default
  var enablesReturnKeyAutomatically: Bool = false

  // MARK: - Focus

  override var canBecomeFirstResponder: Bool {
    mode == .terminal
  }

  override func becomeFirstResponder() -> Bool {
    let ok = super.becomeFirstResponder()
    if ok {
      onFocusChange(["focused": true])
    }
    return ok
  }

  override func resignFirstResponder() -> Bool {
    stopRepeat()
    let wasFirst = isFirstResponder
    let ok = super.resignFirstResponder()
    if ok && wasFirst {
      onFocusChange(["focused": false])
    }
    return ok
  }

  func focusCapture() {
    if mode == .terminal {
      _ = becomeFirstResponder()
    }
  }

  func blurCapture() {
    if isFirstResponder {
      _ = resignFirstResponder()
    }
  }

  // MARK: - UIKeyInput (soft keyboard, and hardware keys this view does not take itself)

  var hasText: Bool {
    // Backspace must always reach deleteBackward, even with nothing typed here.
    true
  }

  func insertText(_ text: String) {
    onText(["text": text])
  }

  func deleteBackward() {
    emitKey("Backspace", code: "Backspace", flags: [])
  }

  // MARK: - Hardware keys (terminal mode)

  override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
    guard mode == .terminal else {
      super.pressesBegan(presses, with: event)
      return
    }
    var rest = Set<UIPress>()
    for press in presses {
      if let key = press.key, let (name, code) = Self.terminalKey(key) {
        handled.insert(press)
        let named = Self.named[key.keyCode] != nil
        let char = named ? "" : key.characters
        emitKey(name, code: code, flags: key.modifierFlags, char: char)
        if named {
          startRepeat(press, name: name, code: code, flags: key.modifierFlags)
        }
      } else {
        rest.insert(press)
      }
    }
    if !rest.isEmpty {
      super.pressesBegan(rest, with: event)
    }
  }

  override func pressesChanged(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
    let rest = presses.subtracting(handled)
    if !rest.isEmpty {
      super.pressesChanged(rest, with: event)
    }
  }

  override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
    stopRepeat(presses)
    let rest = presses.subtracting(handled)
    handled.subtract(presses)
    if !rest.isEmpty {
      super.pressesEnded(rest, with: event)
    }
  }

  override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
    stopRepeat(presses)
    let rest = presses.subtracting(handled)
    handled.subtract(presses)
    if !rest.isEmpty {
      super.pressesCancelled(rest, with: event)
    }
  }

  override func willMove(toWindow newWindow: UIWindow?) {
    super.willMove(toWindow: newWindow)
    if newWindow == nil {
      stopRepeat()
    }
  }

  // MARK: - Key repeat (terminal mode)

  /// Sends `name` again while `press` is held: after repeatDelay, then every repeatInterval.
  private func startRepeat(_ press: UIPress, name: String, code: String, flags: UIKeyModifierFlags) {
    stopRepeat()
    repeatPress = press
    repeatTimer = Timer.scheduledTimer(withTimeInterval: Self.repeatDelay, repeats: false) { [weak self] _ in
      guard let self, self.repeatPress === press else {
        return
      }
      self.repeatTimer = Timer.scheduledTimer(withTimeInterval: Self.repeatInterval, repeats: true) { [weak self] _ in
        self?.emitKey(name, code: code, flags: flags)
      }
    }
  }

  /// Stops the repeat (only when `presses` holds the repeating one, when given).
  private func stopRepeat(_ presses: Set<UIPress>? = nil) {
    if let presses, let press = repeatPress, !presses.contains(press) {
      return
    }
    repeatTimer?.invalidate()
    repeatTimer = nil
    repeatPress = nil
  }

  /// W3C key names of the keys that are not text.
  private static let named: [UIKeyboardHIDUsage: String] = {
    var map: [UIKeyboardHIDUsage: String] = [
      .keyboardEscape: "Escape",
      .keyboardTab: "Tab",
      .keyboardReturnOrEnter: "Enter",
      .keypadEnter: "Enter",
      .keyboardDeleteOrBackspace: "Backspace",
      .keyboardDeleteForward: "Delete",
      .keyboardInsert: "Insert",
      .keyboardUpArrow: "ArrowUp",
      .keyboardDownArrow: "ArrowDown",
      .keyboardLeftArrow: "ArrowLeft",
      .keyboardRightArrow: "ArrowRight",
      .keyboardHome: "Home",
      .keyboardEnd: "End",
      .keyboardPageUp: "PageUp",
      .keyboardPageDown: "PageDown",
    ]
    let fkeys: [UIKeyboardHIDUsage] = [
      .keyboardF1, .keyboardF2, .keyboardF3, .keyboardF4, .keyboardF5, .keyboardF6,
      .keyboardF7, .keyboardF8, .keyboardF9, .keyboardF10, .keyboardF11, .keyboardF12,
    ]
    for (i, usage) in fkeys.enumerated() {
      map[usage] = "F\(i + 1)"
    }
    return map
  }()

  /// The key this view takes itself in terminal mode, or nil to let the text system have it
  /// (printable text, through insertText) or the system (Cmd combinations).
  private static func terminalKey(_ key: UIKey) -> (String, String)? {
    let flags = key.modifierFlags
    if flags.contains(.command) {
      return nil
    }
    if let name = named[key.keyCode] {
      return (name, name)
    }
    if flags.contains(.control) || flags.contains(.alternate) {
      let chars = key.charactersIgnoringModifiers
      if !chars.isEmpty {
        return (chars, "")
      }
    }
    return nil
  }

  // MARK: - Shortcuts mode (key commands from the field inside)

  override var keyCommands: [UIKeyCommand]? {
    guard mode == .shortcuts else {
      return nil
    }
    return captureTab ? Self.shortcutCommands + Self.tabCommands : Self.shortcutCommands
  }

  private static func command(_ input: String, _ flags: UIKeyModifierFlags) -> UIKeyCommand {
    let command = UIKeyCommand(input: input, modifierFlags: flags, action: #selector(KeyCaptureView.onShortcut(_:)))
    command.wantsPriorityOverSystemBehavior = true
    return command
  }

  private static let tabCommands: [UIKeyCommand] = [command("\t", []), command("\t", .shift)]

  private static let shortcutCommands: [UIKeyCommand] = {
    var list: [UIKeyCommand] = []
    func add(_ input: String, _ flags: UIKeyModifierFlags) {
      list.append(command(input, flags))
    }
    // not Option alone: it composes the layout's characters, typed into the field
    for ch in "abcdefghijklmnopqrstuvwxyz [" {
      let input = String(ch)
      add(input, .control)
      add(input, [.control, .alternate])
    }
    add(UIKeyCommand.inputEscape, [])
    let fkeys = [
      UIKeyCommand.f1, UIKeyCommand.f2, UIKeyCommand.f3, UIKeyCommand.f4, UIKeyCommand.f5, UIKeyCommand.f6,
      UIKeyCommand.f7, UIKeyCommand.f8, UIKeyCommand.f9, UIKeyCommand.f10, UIKeyCommand.f11, UIKeyCommand.f12,
    ]
    for f in fkeys {
      add(f, [])
    }
    return list
  }()

  @objc func onShortcut(_ command: UIKeyCommand) {
    guard let input = command.input else {
      return
    }
    let fkeys = [
      UIKeyCommand.f1, UIKeyCommand.f2, UIKeyCommand.f3, UIKeyCommand.f4, UIKeyCommand.f5, UIKeyCommand.f6,
      UIKeyCommand.f7, UIKeyCommand.f8, UIKeyCommand.f9, UIKeyCommand.f10, UIKeyCommand.f11, UIKeyCommand.f12,
    ]
    let name: String
    if input == UIKeyCommand.inputEscape {
      name = "Escape"
    } else if input == "\t" {
      name = "Tab"
    } else if let i = fkeys.firstIndex(of: input) {
      name = "F\(i + 1)"
    } else {
      name = input
    }
    emitKey(name, code: "", flags: command.modifierFlags)
  }

  private func emitKey(_ key: String, code: String, flags: UIKeyModifierFlags, char: String = "") {
    onKey([
      "key": key,
      "code": code,
      "char": char,
      "altGraph": false,
      "ctrl": flags.contains(.control),
      "alt": flags.contains(.alternate),
      "shift": flags.contains(.shift),
      "meta": flags.contains(.command),
    ])
  }
}
