package dev.botjaeger.rift.tunnel

import android.content.ActivityNotFoundException
import android.content.ComponentName
import android.content.Intent
import android.net.Uri
import android.util.Base64
import android.util.Log
import androidx.browser.customtabs.CustomTabsClient
import androidx.browser.customtabs.CustomTabsIntent
import androidx.browser.customtabs.CustomTabsServiceConnection
import expo.modules.kotlin.Promise
import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import java.io.IOException
import java.net.BindException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.atomic.AtomicInteger

private const val TAG = "RiftTunnel"
private const val EXTRA_KEEP_ALIVE = "android.support.customtabs.extra.KEEP_ALIVE"

/**
 * A TCP listener on this phone's loopback (127.0.0.1 only): every connection to it becomes a
 * stream whose bytes JS reads (`onData`, base64) and writes (`write`). JS forwards them to the
 * computer, so a page in the browser tab can load the computer's localhost.
 */
class RiftTunnelModule : Module() {
  private class Stream(val socket: Socket, val writer: ExecutorService)

  @Volatile private var server: ServerSocket? = null
  private val streams = ConcurrentHashMap<Int, Stream>()
  private val nextStream = AtomicInteger(1)
  private var tabConnection: CustomTabsServiceConnection? = null

  override fun definition() = ModuleDefinition {
    Name("RiftTunnel")

    Events("onOpen", "onData", "onEnd", "onClose")

    AsyncFunction("listen") { port: Int ->
      listen(port)
    }

    AsyncFunction("stop") {
      stopAll()
    }

    AsyncFunction("write") { stream: Int, base64: String, promise: Promise ->
      write(stream, Base64.decode(base64, Base64.DEFAULT), promise)
    }

    AsyncFunction("close") { stream: Int ->
      finish(stream)
    }

    AsyncFunction("openTab") { url: String, promise: Promise ->
      openTab(url, promise)
    }.runOnQueue(Queues.MAIN)

    OnDestroy {
      stopAll()
    }
  }

  /**
   * Opens [url] in a custom tab of the browser that shows them, on a custom tabs session whose
   * keep-alive is [KeepAliveService]: the browser binds it while the tab is open, so the app keeps
   * its network behind the tab (Android 15 cuts a background app's network about 5 s after it
   * leaves the front, and with it the tunnel). Resolves true once the tab was asked to open,
   * false when no browser shows custom tabs.
   */
  private fun openTab(url: String, promise: Promise) {
    val activity = appContext.currentActivity ?: return promise.resolve(false)
    val pkg = CustomTabsClient.getPackageName(activity, null) ?: return promise.resolve(false)
    val context = activity.applicationContext
    tabConnection?.let { runCatching { context.unbindService(it) } }
    val connection = object : CustomTabsServiceConnection() {
      override fun onCustomTabsServiceConnected(name: ComponentName, client: CustomTabsClient) {
        client.warmup(0)
        val session = client.newSession(null)
        val tab = CustomTabsIntent.Builder().apply { session?.let { setSession(it) } }.build()
        tab.intent.setPackage(pkg)
        tab.intent.putExtra(EXTRA_KEEP_ALIVE, Intent().setClassName(activity.packageName, KeepAliveService::class.java.name))
        try {
          tab.launchUrl(activity, Uri.parse(url))
          Log.i(TAG, "opened tab in $pkg (session ${session != null})")
          promise.resolve(true)
        } catch (e: ActivityNotFoundException) {
          promise.resolve(false)
        }
      }

      override fun onServiceDisconnected(name: ComponentName?) {}
    }
    tabConnection = connection
    if (!CustomTabsClient.bindCustomTabsService(context, pkg, connection)) {
      tabConnection = null
      promise.resolve(false)
    }
  }

  /** Binds 127.0.0.1:[port]; when it is taken (or 0), an ephemeral port. Returns the bound port. */
  private fun listen(port: Int): Int {
    stopAll()
    // 127.0.0.1 itself: getLoopbackAddress() is ::1 on Android, which a browser asked for
    // http://127.0.0.1 never reaches.
    val loopback = InetAddress.getByAddress(byteArrayOf(127, 0, 0, 1))
    val socket = try {
      ServerSocket(port.coerceIn(0, 65535), 50, loopback)
    } catch (e: BindException) {
      Log.i(TAG, "bind $port failed: ${e.message}")
      ServerSocket(0, 50, loopback)
    }
    server = socket
    Log.i(TAG, "listening on ${loopback.hostAddress}:${socket.localPort}")
    Thread({ acceptLoop(socket) }, "rift-tunnel-accept").apply { isDaemon = true }.start()
    return socket.localPort
  }

  private fun acceptLoop(socket: ServerSocket) {
    while (!socket.isClosed) {
      val conn = try {
        socket.accept()
      } catch (e: IOException) {
        break
      }
      val id = nextStream.getAndIncrement()
      conn.tcpNoDelay = true
      streams[id] = Stream(conn, Executors.newSingleThreadExecutor())
      Log.i(TAG, "open stream $id from ${conn.remoteSocketAddress}")
      sendEvent("onOpen", mapOf("stream" to id))
      Thread({ readLoop(id, conn) }, "rift-tunnel-$id").apply { isDaemon = true }.start()
    }
  }

  private fun readLoop(id: Int, conn: Socket) {
    val buf = ByteArray(64 * 1024)
    try {
      val input = conn.getInputStream()
      while (true) {
        val n = input.read(buf)
        if (n < 0) {
          // The peer finished sending (half-close): the stream stays writable until close().
          if (streams.containsKey(id)) {
            Log.i(TAG, "stream $id end")
            sendEvent("onEnd", mapOf("stream" to id))
          }
          return
        }
        if (n == 0) continue
        Log.i(TAG, "stream $id read $n bytes")
        sendEvent("onData", mapOf("stream" to id, "data" to Base64.encodeToString(buf, 0, n, Base64.NO_WRAP)))
      }
    } catch (e: IOException) {
      // reset or closed by finish()
    }
    finish(id)
  }

  /**
   * Writes [bytes] after the stream's earlier writes; resolves once they are written (JS
   * acknowledges them to the computer then, so a browser that reads slowly holds the computer's
   * window, not a growing queue here), false when the stream is gone.
   */
  private fun write(id: Int, bytes: ByteArray, promise: Promise) {
    val s = streams[id] ?: return promise.resolve(false)
    try {
      s.writer.execute {
        try {
          s.socket.getOutputStream().write(bytes)
          s.socket.getOutputStream().flush()
          promise.resolve(true)
        } catch (e: IOException) {
          finish(id)
          promise.resolve(false)
        }
      }
    } catch (e: RejectedExecutionException) {
      promise.resolve(false) // finished meanwhile
    }
  }

  /** Ends a stream once: its queued writes go out first, then the socket closes. */
  private fun finish(id: Int) {
    val s = streams.remove(id) ?: return
    s.writer.execute {
      try {
        s.socket.shutdownOutput()
      } catch (e: IOException) {
        // already gone
      }
      try {
        s.socket.close()
      } catch (e: IOException) {
        // already gone
      }
    }
    s.writer.shutdown()
    Log.i(TAG, "close stream $id")
    sendEvent("onClose", mapOf("stream" to id))
  }

  private fun stopAll() {
    server?.let {
      Log.i(TAG, "stopped listening on 127.0.0.1:${it.localPort}")
      try {
        it.close()
      } catch (e: IOException) {
        // already closed
      }
    }
    server = null
    for (id in streams.keys.toList()) finish(id)
  }
}
