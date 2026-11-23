package dev.botjaeger.rift.tunnel

import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.IBinder
import android.util.Log

/**
 * The custom tab's keep-alive (`android.support.customtabs.extra.KEEP_ALIVE`): the browser binds
 * to it while the tab is open, which ranks the app with the browser in front. Without it Android
 * 15 cuts a background app's network about 5 s after the tab covers it, and with it the tunnel.
 */
class KeepAliveService : Service() {
  private val binder = Binder()

  override fun onBind(intent: Intent?): IBinder {
    Log.i("RiftTunnel", "keep-alive bound")
    return binder
  }

  override fun onUnbind(intent: Intent?): Boolean {
    Log.i("RiftTunnel", "keep-alive unbound")
    return false
  }
}
