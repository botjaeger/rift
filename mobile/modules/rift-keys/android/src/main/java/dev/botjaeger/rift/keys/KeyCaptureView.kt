package dev.botjaeger.rift.keys

import android.content.Context
import android.graphics.Rect
import android.text.InputType
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView

/**
 * Terminal mode: a focusable view with its own input connection (soft keyboard) and key
 * handling (hardware keys with their meta state and the character the layout composes, for
 * AltGr). Shortcuts mode: the view wraps the command field and takes from it, in
 * dispatchKeyEvent, only ctrl/alt combinations that are keys (an AltGr / Alt character such as
 * "@" or "|" is typed into the field), Escape, F-keys, a hardware keyboard's Return and
 * Shift+Return, and, with captureTab, Tab / Shift+Tab.
 * Meta (Cmd / Search) combinations are left to the system in both.
 *
 * The soft keyboard gets a visible-password text field (no suggestions, no autocorrect, no
 * composing; most keyboards show their number row): every character is committed at once, and
 * any keyboard and language can type, unlike TYPE_NULL, which makes many keyboards fall back
 * to raw key events and drop non-ASCII input.
 */
class KeyCaptureView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  private val onText by EventDispatcher()
  private val onKey by EventDispatcher()
  private val onFocusChange by EventDispatcher()
  private val onPaste by EventDispatcher()

  private var terminal = false

  /** Shortcuts mode: Tab / Shift+Tab go to the pane (else they move the focus). */
  var captureTab = false

  /** Key codes whose ACTION_DOWN this view consumed: their ACTION_UP is consumed too. */
  private val consumed = HashSet<Int>()

  private val imm: InputMethodManager?
    get() = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager

  /**
   * React Native lays out the children. ExpoView is a LinearLayout, whose own pass (run when a
   * child asks for layout, e.g. the command field) stacked the first child over the whole view
   * and pushed the command field out of sight.
   */
  override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) = Unit

  fun setMode(isTerminal: Boolean) {
    terminal = isTerminal
    isFocusable = isTerminal
    isFocusableInTouchMode = isTerminal
    // TalkBack reads the screen rows inside, not this view as one element folding them
    importantForAccessibility =
      if (isTerminal) View.IMPORTANT_FOR_ACCESSIBILITY_NO else View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
    if (!isTerminal && isFocused) blurCapture()
  }

  fun focusCapture() {
    if (!terminal) return
    if (requestFocus()) imm?.showSoftInput(this, 0)
  }

  fun blurCapture() {
    imm?.hideSoftInputFromWindow(windowToken, 0)
    if (isFocused) clearFocus()
  }

  override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
    super.onFocusChanged(gainFocus, direction, previouslyFocusedRect)
    if (!terminal) return
    // Losing focus (leaving the terminal view, which unfocuses the view before it detaches)
    // takes the keyboard along: it would otherwise stay up with nothing to type into. A text
    // field that takes the focus shows its own.
    if (!gainFocus) imm?.hideSoftInputFromWindow(windowToken, 0)
    onFocusChange(mapOf("focused" to gainFocus))
  }

  // --- soft keyboard -------------------------------------------------------------------------

  override fun onCheckIsTextEditor(): Boolean = terminal

  override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
    if (!terminal) return null
    outAttrs.inputType = InputType.TYPE_CLASS_TEXT or
      InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or
      InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
    outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or
      EditorInfo.IME_FLAG_NO_FULLSCREEN or
      EditorInfo.IME_ACTION_NONE
    return TerminalConnection()
  }

  private inner class TerminalConnection : BaseInputConnection(this@KeyCaptureView, false) {
    /** Text a keyboard is still composing; sent once it is committed or finished. */
    private var composing = ""

    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
      composing = ""
      if (!text.isNullOrEmpty()) emitText(text.toString())
      return true
    }

    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
      composing = text?.toString() ?: ""
      return true
    }

    override fun finishComposingText(): Boolean {
      if (composing.isNotEmpty()) emitText(composing)
      composing = ""
      return true
    }

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
      if (composing.isNotEmpty()) {
        composing = composing.dropLast(beforeLength)
        return true
      }
      repeat(beforeLength) { emitKey("Backspace", 0) }
      return true
    }

    override fun performEditorAction(actionCode: Int): Boolean {
      emitKey("Enter", 0)
      return true
    }
  }

  // --- keys --------------------------------------------------------------------------------------

  override fun dispatchKeyEvent(event: KeyEvent): Boolean {
    when (event.action) {
      KeyEvent.ACTION_UP -> if (consumed.remove(event.keyCode)) return true
      KeyEvent.ACTION_DOWN -> {
        val handled = if (terminal) isFocused && terminalKey(event) else shortcutKey(event)
        if (handled) {
          consumed.add(event.keyCode)
          return true
        }
      }
    }
    return super.dispatchKeyEvent(event)
  }

  /** Terminal mode: every key but Meta combinations; printable keys are text. */
  private fun terminalKey(e: KeyEvent): Boolean {
    // the keyboard's Paste key, and Ctrl+Shift+V as in a desktop terminal (Ctrl+V stays the program's)
    if (e.isMetaPressed) return false
    if (e.keyCode == KeyEvent.KEYCODE_PASTE || (e.keyCode == KeyEvent.KEYCODE_V && e.isCtrlPressed && e.isShiftPressed)) {
      // a key held down pastes once
      if (e.repeatCount == 0) onPaste(emptyMap<String, Any>())
      return true
    }
    NAMED[e.keyCode]?.let {
      emitKey(it, e.metaState)
      return true
    }
    if (e.isCtrlPressed || e.isAltPressed) {
      val ch = e.getUnicodeChar(e.metaState and KeyEvent.META_SHIFT_MASK)
      val composed = composedChar(e)
      if (ch <= 0 && composed.isEmpty()) return false
      emitKey(if (ch > 0) String(Character.toChars(ch)) else composed, e.metaState, composed)
      return true
    }
    val ch = e.unicodeChar
    if (ch <= 0 || (ch and KeyCharacterMap.COMBINING_ACCENT) != 0) return false
    emitText(String(Character.toChars(ch)))
    return true
  }

  /**
   * Shortcuts mode: ctrl/alt combinations that are keys (as keyboard.ts names them: a letter or
   * space, ctrl+[, ctrl+@ / ctrl+2), Escape, F-keys, and Tab / Shift+Tab with captureTab. A
   * character composed with AltGr / Alt (keyboard.ts `composedText`) is left to the field.
   */
  private fun shortcutKey(e: KeyEvent): Boolean {
    if (e.isMetaPressed) return false
    val code = e.keyCode
    // Shift+Return in the field: a new line in the message. Plain Return from a physical keyboard
    // sends it; the on-screen keyboard's Return (a virtual device) is left to the field, a new line.
    if (code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_NUMPAD_ENTER) {
      val plain = !e.isCtrlPressed && !e.isAltPressed
      val physical = e.deviceId != KeyCharacterMap.VIRTUAL_KEYBOARD && e.device?.isVirtual == false
      if (e.isShiftPressed || (plain && physical)) {
        emitKey("Enter", e.metaState)
        return true
      }
    }
    if (code == KeyEvent.KEYCODE_TAB && !captureTab) return false
    if (code == KeyEvent.KEYCODE_ESCAPE || code == KeyEvent.KEYCODE_TAB ||
      (code >= KeyEvent.KEYCODE_F1 && code <= KeyEvent.KEYCODE_F12)
    ) {
      emitKey(NAMED[code]!!, e.metaState)
      return true
    }
    if (e.isCtrlPressed || e.isAltPressed) {
      val ch = e.getUnicodeChar(e.metaState and KeyEvent.META_SHIFT_MASK)
      if (ch <= 0) return false
      val base = String(Character.toChars(ch))
      val composed = composedChar(e)
      val altGraph = e.metaState and KeyEvent.META_ALT_RIGHT_ON != 0
      if (composed.isNotEmpty() && (altGraph || (!e.isCtrlPressed && composed != base))) return false
      val lower = base.lowercase()
      val isKey = (lower.length == 1 && lower[0] in 'a'..'z') || lower == " " ||
        (e.isCtrlPressed && (lower == "[" || lower == "@" || lower == "2"))
      if (!isKey) return false
      emitKey(base, e.metaState, composed)
      return true
    }
    return false
  }

  /** The printable character the layout composes with every modifier held (AltGr+Q "@"), or "". */
  private fun composedChar(e: KeyEvent): String {
    val c = e.getUnicodeChar(e.metaState)
    if (c <= 0 || (c and KeyCharacterMap.COMBINING_ACCENT) != 0) return ""
    if (c < 0x20 || c in 0x7f..0x9f) return ""
    return String(Character.toChars(c))
  }

  private fun emitText(text: String) {
    onText(mapOf("text" to text))
  }

  private fun emitKey(key: String, meta: Int, composed: String = "") {
    onKey(
      mapOf(
        "key" to key,
        "code" to "",
        "char" to composed,
        "altGraph" to (meta and KeyEvent.META_ALT_RIGHT_ON != 0),
        "ctrl" to (meta and KeyEvent.META_CTRL_ON != 0),
        "alt" to (meta and KeyEvent.META_ALT_ON != 0),
        "shift" to (meta and KeyEvent.META_SHIFT_ON != 0),
        "meta" to (meta and KeyEvent.META_META_ON != 0),
      ),
    )
  }

  companion object {
    /** Android key codes → W3C key names, for the keys that are not text. */
    private val NAMED: Map<Int, String> = buildMap {
      put(KeyEvent.KEYCODE_ESCAPE, "Escape")
      put(KeyEvent.KEYCODE_TAB, "Tab")
      put(KeyEvent.KEYCODE_ENTER, "Enter")
      put(KeyEvent.KEYCODE_NUMPAD_ENTER, "Enter")
      put(KeyEvent.KEYCODE_DEL, "Backspace")
      put(KeyEvent.KEYCODE_FORWARD_DEL, "Delete")
      put(KeyEvent.KEYCODE_INSERT, "Insert")
      put(KeyEvent.KEYCODE_DPAD_UP, "ArrowUp")
      put(KeyEvent.KEYCODE_DPAD_DOWN, "ArrowDown")
      put(KeyEvent.KEYCODE_DPAD_LEFT, "ArrowLeft")
      put(KeyEvent.KEYCODE_DPAD_RIGHT, "ArrowRight")
      put(KeyEvent.KEYCODE_MOVE_HOME, "Home")
      put(KeyEvent.KEYCODE_MOVE_END, "End")
      put(KeyEvent.KEYCODE_PAGE_UP, "PageUp")
      put(KeyEvent.KEYCODE_PAGE_DOWN, "PageDown")
      for (i in 0 until 12) put(KeyEvent.KEYCODE_F1 + i, "F${i + 1}")
    }
  }
}
