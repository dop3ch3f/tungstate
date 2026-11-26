package com.ifeanyiibekie.tungstate.photos

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.annotation.RequiresApi
import androidx.core.app.NotificationCompat

/** What the notification says, as the app's Rust words it. */
data class Notice(val title: String, val text: String, val permille: Int, val waiting: Boolean)

/**
 * Keeps a transfer the person started going with the screen off or the app
 * in the background, and shows it in the shade with a Stop button.
 *
 * The transfer itself runs on the app's own Rust threads. All this does is
 * keep the process at foreground priority and awake:
 *  - Android 14 and later: a user-initiated data transfer job, the API made
 *    for exactly this. No time limit, unlike a data-sync service from 15 on.
 *  - Before 14, or with no network to give the job (a copy to the card in
 *    airplane mode): a data-sync foreground service.
 *  - Either way: a partial wake lock, so the processor does not sleep with
 *    the screen, and a Wi-Fi lock, so the radio does not doze.
 *
 * One transfer at a time, as the engine runs them, so one shared state.
 */
object TransferKeeper {
  private const val TAG = "TungstateKeeper"
  private const val CHANNEL = "transfers"
  const val NOTE_ID = 4711
  private const val JOB_ID = 4711
  const val ACTION_STOP = "com.ifeanyiibekie.tungstate.photos.STOP"

  /** Stop was pressed on the notification; read (and cleared) on the next update. */
  @Volatile private var stopAsked = false
  @Volatile var notice: Notice? = null
    private set

  private var wake: PowerManager.WakeLock? = null
  private var wifi: WifiManager.WifiLock? = null
  private var job: Pair<JobService, JobParameters>? = null
  private var service: Service? = null
  private var usingJob = false

  @Synchronized
  fun begin(context: Context, first: Notice) {
    val app = context.applicationContext
    notice = first
    stopAsked = false
    channel(app)
    holdLocks(app)
    usingJob = Build.VERSION.SDK_INT >= 34 && online(app) && scheduleJob(app)
    if (!usingJob) startService(app)
  }

  /** Shows `next`, and answers whether Stop was pressed since the last time. */
  @Synchronized
  fun update(context: Context, next: Notice): Boolean {
    notice = next
    val app = context.applicationContext
    // Even while the system has the job stopped (the network went), so the
    // shade says it is waiting rather than showing the last figure for ever.
    try {
      manager(app).notify(NOTE_ID, build(app, next))
    } catch (e: SecurityException) {
      // Notifications refused: the transfer carries on unseen.
    }
    val stop = stopAsked
    stopAsked = false
    return stop
  }

  @Synchronized
  fun end(context: Context) {
    val app = context.applicationContext
    notice = null
    job?.let { (svc, params) -> svc.jobFinished(params, false) }
    job = null
    if (usingJob) app.getSystemService(JobScheduler::class.java)?.cancel(JOB_ID)
    service?.let {
      it.stopForeground(Service.STOP_FOREGROUND_REMOVE)
      it.stopSelf()
    }
    service = null
    manager(app).cancel(NOTE_ID)
    wake?.takeIf { it.isHeld }?.release()
    wake = null
    wifi?.takeIf { it.isHeld }?.release()
    wifi = null
  }

  fun askStop() {
    stopAsked = true
  }

  /** The app is starting. A notification or job left by a run whose process
   *  the system killed would otherwise sit in the shade saying "Copying" for
   *  ever; the app offers Resume for that run instead. */
  @Synchronized
  fun clearStale(context: Context) {
    if (notice != null) return
    val app = context.applicationContext
    manager(app).cancel(NOTE_ID)
    if (Build.VERSION.SDK_INT >= 34) app.getSystemService(JobScheduler::class.java)?.cancel(JOB_ID)
  }

  // ── the job (Android 14+) ────────────────────────────────────────────────
  @RequiresApi(34)
  private fun scheduleJob(app: Context): Boolean = try {
    val info = JobInfo.Builder(JOB_ID, ComponentName(app, TransferJobService::class.java))
      .setUserInitiated(true)
      // A user-initiated job must say what network it needs. Any: the engine
      // itself decides whether the far side answers.
      .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
      .build()
    app.getSystemService(JobScheduler::class.java).schedule(info) == JobScheduler.RESULT_SUCCESS
  } catch (e: Exception) {
    Log.w(TAG, "could not schedule the transfer job; using a service", e)
    false
  }

  @Synchronized
  fun jobStarted(svc: JobService, params: JobParameters) {
    job = svc to params
    val current = notice
    if (current == null) {
      // Ended before the system got round to starting it.
      svc.jobFinished(params, false)
      job = null
      return
    }
    if (Build.VERSION.SDK_INT >= 34) {
      // Detached: if the network drops and the system stops the job, the
      // notification stays to say the transfer is waiting for it.
      svc.setNotification(params, NOTE_ID, build(svc, current), JobService.JOB_END_NOTIFICATION_POLICY_DETACH)
    }
  }

  /** The system stopped the job, usually because the network went. Still
   *  running: ask for it again, which brings it back when the network does. */
  @Synchronized
  fun jobStopped(): Boolean {
    job = null
    val again = notice != null
    if (again) Log.i(TAG, "the transfer job was stopped; asking for it again")
    return again
  }

  // ── the service (before 14, or offline) ──────────────────────────────────
  private fun startService(app: Context) {
    val intent = Intent(app, TransferService::class.java)
    try {
      if (Build.VERSION.SDK_INT >= 26) app.startForegroundService(intent) else app.startService(intent)
    } catch (e: Exception) {
      // Started from the foreground, so this is rare; the wake lock still
      // holds while the app stays open.
      Log.w(TAG, "could not start the transfer service", e)
    }
  }

  @Synchronized
  fun serviceStarted(svc: Service) {
    val current = notice
    if (current == null) {
      svc.stopSelf()
      return
    }
    service = svc
    val note = build(svc, current)
    if (Build.VERSION.SDK_INT >= 29) {
      svc.startForeground(NOTE_ID, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
    } else {
      svc.startForeground(NOTE_ID, note)
    }
  }

  // ── the rest ─────────────────────────────────────────────────────────────
  private fun holdLocks(app: Context) {
    if (wake == null) {
      val power = app.getSystemService(Context.POWER_SERVICE) as PowerManager
      // A ceiling rather than forever, should a run somehow never end: twelve
      // hours is more than any phone's worth of photos over Wi-Fi.
      wake = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tungstate:transfer").apply {
        setReferenceCounted(false)
        acquire(12 * 60 * 60 * 1000L)
      }
    }
    if (wifi == null) {
      val manager = app.getSystemService(Context.WIFI_SERVICE) as WifiManager
      @Suppress("DEPRECATION")
      val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF
      wifi = manager.createWifiLock(mode, "tungstate:transfer").apply {
        setReferenceCounted(false)
        acquire()
      }
    }
  }

  private fun online(app: Context): Boolean {
    val connectivity = app.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    return connectivity.activeNetwork != null
  }

  private fun manager(app: Context) = app.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

  private fun channel(app: Context) {
    if (Build.VERSION.SDK_INT < 26) return
    // Low: no sound and no pop-up for a bar that updates every second.
    val channel = NotificationChannel(CHANNEL, "Transfers", NotificationManager.IMPORTANCE_LOW).apply {
      description = "Copies and moves kept going while the screen is off"
    }
    manager(app).createNotificationChannel(channel)
  }

  private fun build(context: Context, n: Notice): Notification {
    val open = context.packageManager.getLaunchIntentForPackage(context.packageName)?.let {
      it.flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT
      PendingIntent.getActivity(context, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
    val stop = PendingIntent.getBroadcast(
      context,
      1,
      Intent(context, StopReceiver::class.java).setAction(ACTION_STOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    return NotificationCompat.Builder(context, CHANNEL)
      // The system's own upload arrow, which animates in the status bar.
      .setSmallIcon(if (n.waiting) android.R.drawable.stat_sys_warning else android.R.drawable.stat_sys_upload)
      .setContentTitle(n.title)
      .setContentText(n.text)
      .setStyle(NotificationCompat.BigTextStyle().bigText(n.text))
      .setProgress(1000, n.permille, false)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setSilent(true)
      .setCategory(NotificationCompat.CATEGORY_PROGRESS)
      .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
      .setContentIntent(open)
      .addAction(0, "Stop", stop)
      .build()
  }
}

/** The user-initiated data transfer job: holds no work, only the process. */
class TransferJobService : JobService() {
  override fun onStartJob(params: JobParameters): Boolean {
    TransferKeeper.jobStarted(this, params)
    return true
  }

  override fun onStopJob(params: JobParameters): Boolean = TransferKeeper.jobStopped()
}

/** The data-sync foreground service, for Android 13 and earlier. */
class TransferService : Service() {
  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    TransferKeeper.serviceStarted(this)
    // Not restarted after the process dies: a transfer is resumed by the
    // person, from the app, never on its own.
    return START_NOT_STICKY
  }

  override fun onBind(intent: Intent?): IBinder? = null
}

/** Stop on the notification: the run stops after the file in hand. */
class StopReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    if (intent.action == TransferKeeper.ACTION_STOP) TransferKeeper.askStop()
  }
}
