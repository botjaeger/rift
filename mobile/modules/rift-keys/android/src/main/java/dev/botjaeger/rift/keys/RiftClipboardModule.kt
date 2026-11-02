package dev.botjaeger.rift.keys

import android.content.ClipboardManager
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import java.io.ByteArrayOutputStream
import java.io.InputStream

/**
 * The file on the clipboard, for a paste the user asked for: the clip's content URI, read within
 * the upload limit. Android shows its "pasted from" notice; that cannot be avoided. The clip is
 * another app's input: only a content URI of another app is read, never a file URI or this app's
 * own provider, which would hand this app's private files to the computer.
 */
class RiftClipboardModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("RiftClipboard")

    AsyncFunction("clipFile") {
      clipFile()
    }
  }

  private fun clipFile(): Map<String, Any> {
    val context = appContext.reactContext ?: return mapOf("status" to "unreadable")
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
      ?: return mapOf("status" to "none")
    val clip = clipboard.primaryClip ?: return mapOf("status" to "none")
    if (clip.itemCount == 0) return mapOf("status" to "none")
    val uri = clip.getItemAt(0).uri ?: return mapOf("status" to "none")
    if (uri.scheme != "content" || uri.authority?.startsWith(context.packageName) != false) {
      return mapOf("status" to "unreadable")
    }
    return try {
      val bytes = context.contentResolver.openInputStream(uri)?.use { readWithin(it) }
        ?: return mapOf("status" to "unreadable")
      if (bytes.size > MAX_BYTES) {
        mapOf("status" to "too_large")
      } else {
        mapOf(
          "status" to "ok",
          "name" to displayName(context, uri),
          "data" to Base64.encodeToString(bytes, Base64.NO_WRAP),
        )
      }
    } catch (_: Exception) {
      mapOf("status" to "unreadable")
    }
  }

  /** The stream's bytes, stopping one past the limit. */
  private fun readWithin(input: InputStream): ByteArray {
    val out = ByteArrayOutputStream()
    val buffer = ByteArray(64 * 1024)
    while (out.size() <= MAX_BYTES) {
      val n = input.read(buffer)
      if (n < 0) break
      out.write(buffer, 0, n)
    }
    return out.toByteArray()
  }

  /** The name the provider gives, for its extension only; "" when it gives none. */
  private fun displayName(context: Context, uri: Uri): String {
    context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
      if (c.moveToFirst() && !c.isNull(0)) return c.getString(0) ?: ""
    }
    return ""
  }

  private companion object {
    /** MAX_UPLOAD_BYTES of the protocol. */
    const val MAX_BYTES = 10 * 1024 * 1024
  }
}
