package com.ifeanyiibekie.tungstate.photos

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.view.WindowManager
import androidx.activity.result.ActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.core.content.ContextCompat
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.util.Calendar
import java.util.concurrent.Executors

@InvokeArg
class Filter {
  /** "all", "videos" or "screenshots". */
  var kind: String = "all"
  /** Only what is older than this many days; zero means everything. */
  var olderThanDays: Int = 0
}

@InvokeArg
class ExportArgs {
  lateinit var id: String
  lateinit var path: String
}

@InvokeArg
class RemoveArgs {
  var ids: Array<String> = emptyArray()
}

@InvokeArg
class AwakeArgs {
  var on: Boolean = false
}

/** One photo or video as the library holds it. */
private class Row(
  val uri: Uri,
  val name: String,
  val size: Long,
  val at: Long,
  val favourite: Boolean,
  val video: Boolean,
  val screenshot: Boolean,
)

// Android 13 split photo access into images and video; Android 14 added
// "select photos", which grants only what the person picked. Before 13 it
// is the one storage permission. Asking for a permission an older Android
// does not know is ignored, so one alias covers 13 and later.
//
// ACCESS_MEDIA_LOCATION rides along: without it Android hands out photos
// with their location stripped, and a copy missing where it was taken is
// not a copy worth clearing the original for. It shows no prompt of its own.
@TauriPlugin(
  permissions = [
    Permission(
      strings = [
        Manifest.permission.READ_MEDIA_IMAGES,
        Manifest.permission.READ_MEDIA_VIDEO,
        Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED,
        Manifest.permission.ACCESS_MEDIA_LOCATION,
      ],
      alias = "media",
    ),
    Permission(
      strings = [Manifest.permission.READ_EXTERNAL_STORAGE, Manifest.permission.ACCESS_MEDIA_LOCATION],
      alias = "storage",
    ),
  ],
)
class PhotosPlugin(private val activity: Activity) : Plugin(activity) {
  private val alias = if (Build.VERSION.SDK_INT >= 33) "media" else "storage"

  // Commands arrive on the main thread. Reading the library and copying out
  // of it can take a while, so it happens here, one job at a time.
  private val worker = Executors.newSingleThreadExecutor()

  private fun granted(permission: String) =
    ContextCompat.checkSelfPermission(activity, permission) == PackageManager.PERMISSION_GRANTED

  /** "full", "limited", "denied" or "ask". */
  private fun state(): String {
    if (Build.VERSION.SDK_INT >= 33) {
      if (granted(Manifest.permission.READ_MEDIA_IMAGES) || granted(Manifest.permission.READ_MEDIA_VIDEO)) return "full"
      if (Build.VERSION.SDK_INT >= 34 && granted(Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)) return "limited"
    } else if (granted(Manifest.permission.READ_EXTERNAL_STORAGE)) {
      return "full"
    }
    // Tauri remembers whether it has asked: never asked is "ask", asked and
    // refused is "denied", which only the system's Settings can undo.
    return if (getPermissionState(alias) == PermissionState.DENIED) "denied" else "ask"
  }

  private fun answer(invoke: Invoke) {
    val ret = JSObject()
    ret.put("access", state())
    invoke.resolve(ret)
  }

  /** Runs `job` on the worker, turning anything it throws into a rejection. */
  private fun offMain(invoke: Invoke, fallback: String, job: () -> Unit) {
    worker.execute {
      try {
        job()
      } catch (e: Exception) {
        invoke.reject(e.message ?: fallback)
      }
    }
  }

  @Command
  fun access(invoke: Invoke) = answer(invoke)

  @Command
  fun ask(invoke: Invoke) {
    if (state() == "full") return answer(invoke)
    requestPermissionForAlias(alias, invoke, "asked")
  }

  @PermissionCallback
  private fun asked(invoke: Invoke) = answer(invoke)

  @Command
  fun summary(invoke: Invoke) {
    val filter = invoke.parseArgs(Filter::class.java)
    offMain(invoke, "could not read the photo library") { invoke.resolve(count(filter)) }
  }

  @Command
  fun list(invoke: Invoke) {
    val filter = invoke.parseArgs(Filter::class.java)
    offMain(invoke, "could not read the photo library") {
      val calendar = Calendar.getInstance()
      val assets = JSArray()
      scan(filter) { row ->
        calendar.timeInMillis = row.at
        val asset = JSObject()
        asset.put("id", row.uri.toString())
        asset.put("name", row.name)
        asset.put("size", row.size)
        asset.put("takenMs", row.at)
        asset.put("year", calendar.get(Calendar.YEAR))
        asset.put("kind", if (row.video) "video" else if (row.screenshot) "screenshot" else "photo")
        asset.put("favourite", row.favourite)
        assets.put(asset)
      }
      val ret = JSObject()
      ret.put("assets", assets)
      invoke.resolve(ret)
    }
  }

  /** Copies one photo or video, byte for byte, into a file. */
  @Command
  fun export(invoke: Invoke) {
    val args = invoke.parseArgs(ExportArgs::class.java)
    offMain(invoke, "could not copy it out of the photo library") {
      var uri = Uri.parse(args.id)
      if (Build.VERSION.SDK_INT >= 29) {
        // Refused rather than copied without its location: see the
        // permission note above.
        if (!granted(Manifest.permission.ACCESS_MEDIA_LOCATION)) {
          throw IllegalStateException("Tungstate may not read where this was taken, so it is left on the phone")
        }
        uri = MediaStore.setRequireOriginal(uri)
      }
      val input = activity.contentResolver.openInputStream(uri)
        ?: throw IllegalStateException("the photo library would not open it")
      input.use { from -> File(args.path).outputStream().use { to -> from.copyTo(to, 1 shl 20) } }
      invoke.resolve()
    }
  }

  /**
   * Asks the person, through Android's own prompt, to delete these for good.
   * Answers how many are gone, counted afterwards rather than assumed.
   * Android 10 and earlier cannot be asked; they answer `supported: false`.
   */
  @Command
  fun remove(invoke: Invoke) {
    val args = invoke.parseArgs(RemoveArgs::class.java)
    if (Build.VERSION.SDK_INT < 30) return resolveRemoved(invoke, 0, false)
    if (args.ids.isEmpty()) return resolveRemoved(invoke, 0, true)
    val uris = args.ids.map(Uri::parse)
    val request = MediaStore.createDeleteRequest(activity.contentResolver, uris)
    startIntentSenderForResult(invoke, IntentSenderRequest.Builder(request.intentSender).build(), "removed")
  }

  @ActivityCallback
  private fun removed(invoke: Invoke, result: ActivityResult) {
    val args = invoke.parseArgs(RemoveArgs::class.java)
    if (result.resultCode != Activity.RESULT_OK) return resolveRemoved(invoke, 0, true)
    offMain(invoke, "could not tell what was removed") {
      val left = args.ids.count { id -> stillThere(Uri.parse(id)) }
      resolveRemoved(invoke, args.ids.size - left, true)
    }
  }

  private fun stillThere(uri: Uri): Boolean =
    activity.contentResolver.query(uri, arrayOf(MediaStore.MediaColumns._ID), null, null, null)
      ?.use { it.moveToFirst() } ?: false

  private fun resolveRemoved(invoke: Invoke, removed: Int, supported: Boolean) {
    val ret = JSObject()
    ret.put("removed", removed)
    ret.put("supported", supported)
    invoke.resolve(ret)
  }

  /** Keeps the screen on while a run is open, so the phone does not lock on it. */
  @Command
  fun keepAwake(invoke: Invoke) {
    val args = invoke.parseArgs(AwakeArgs::class.java)
    activity.runOnUiThread {
      if (args.on) activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      else activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      invoke.resolve()
    }
  }

  /** Every photo and video the filter covers, favourites included. */
  private fun scan(filter: Filter, visit: (Row) -> Unit) {
    val image = MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE
    val video = MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
    val type = MediaStore.Files.FileColumns.MEDIA_TYPE
    // Where the file sits decides whether it is a screenshot: Android keeps
    // them in a Screenshots folder and marks them no other way.
    val where = if (Build.VERSION.SDK_INT >= 29) MediaStore.MediaColumns.RELATIVE_PATH else MediaStore.MediaColumns.DATA
    val selection = when (filter.kind) {
      "videos" -> "$type = $video"
      "screenshots" -> "$type = $image AND $where LIKE '%Screenshots%'"
      else -> "$type IN ($image, $video)"
    }
    val projection = mutableListOf(
      MediaStore.MediaColumns._ID,
      MediaStore.MediaColumns.DISPLAY_NAME,
      MediaStore.MediaColumns.SIZE,
      MediaStore.MediaColumns.DATE_TAKEN,
      MediaStore.MediaColumns.DATE_MODIFIED,
      type,
      where,
    )
    if (Build.VERSION.SDK_INT >= 30) projection.add(MediaStore.MediaColumns.IS_FAVORITE)

    val cutoff = if (filter.olderThanDays > 0) System.currentTimeMillis() - filter.olderThanDays * 86_400_000L else Long.MAX_VALUE

    activity.contentResolver.query(
      MediaStore.Files.getContentUri("external"), projection.toTypedArray(), selection, null, null,
    )?.use { rows ->
      val id = rows.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
      val name = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
      val size = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
      val taken = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_TAKEN)
      val modified = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
      val kind = rows.getColumnIndexOrThrow(type)
      val place = rows.getColumnIndexOrThrow(where)
      val favourite = if (Build.VERSION.SDK_INT >= 30) rows.getColumnIndexOrThrow(MediaStore.MediaColumns.IS_FAVORITE) else -1
      while (rows.moveToNext()) {
        // When the photo was taken, or failing that when the file last
        // changed (seconds, where date taken is milliseconds).
        val at = if (!rows.isNull(taken) && rows.getLong(taken) > 0) rows.getLong(taken) else rows.getLong(modified) * 1000
        if (at > cutoff) continue
        val isVideo = rows.getInt(kind) == video
        // Per kind, because that is the address the photo library itself
        // answers to for reading and deleting.
        val collection = if (isVideo) MediaStore.Video.Media.getContentUri("external") else MediaStore.Images.Media.getContentUri("external")
        visit(
          Row(
            uri = ContentUris.withAppendedId(collection, rows.getLong(id)),
            name = rows.getString(name) ?: "",
            size = rows.getLong(size),
            at = at,
            favourite = favourite >= 0 && rows.getInt(favourite) == 1,
            video = isVideo,
            screenshot = !isVideo && (rows.getString(place) ?: "").contains("Screenshots"),
          ),
        )
      }
    }
  }

  private fun count(filter: Filter): JSObject {
    // Sorted by year, so the screen lists newest first without sorting again.
    val years = sortedMapOf<Int, LongArray>(compareByDescending { it })
    var favCount = 0L
    var favBytes = 0L
    val calendar = Calendar.getInstance()
    scan(filter) { row ->
      if (row.favourite) {
        favCount++
        favBytes += row.size
      } else {
        calendar.timeInMillis = row.at
        val tally = years.getOrPut(calendar.get(Calendar.YEAR)) { LongArray(2) }
        tally[0]++
        tally[1] += row.size
      }
    }

    val list = JSArray()
    for ((year, tally) in years) {
      val row = JSObject()
      row.put("year", year)
      row.put("count", tally[0])
      row.put("bytes", tally[1])
      list.put(row)
    }
    val favourites = JSObject()
    favourites.put("count", favCount)
    favourites.put("bytes", favBytes)
    val ret = JSObject()
    ret.put("years", list)
    ret.put("favourites", favourites)
    return ret
  }
}
