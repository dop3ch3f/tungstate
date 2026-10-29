package com.ifeanyiibekie.tungstate.photos

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import android.provider.MediaStore
import androidx.core.content.ContextCompat
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.Calendar

@InvokeArg
class Filter {
  /** "all", "videos" or "screenshots". */
  var kind: String = "all"
  /** Only what is older than this many days; zero means everything. */
  var olderThanDays: Int = 0
}

// Android 13 split photo access into images and video; Android 14 added
// "select photos", which grants only what the person picked. Before 13 it
// is the one storage permission. Asking for a permission an older Android
// does not know is ignored, so one alias covers 13 and later.
@TauriPlugin(
  permissions = [
    Permission(
      strings = [
        Manifest.permission.READ_MEDIA_IMAGES,
        Manifest.permission.READ_MEDIA_VIDEO,
        Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED,
      ],
      alias = "media",
    ),
    Permission(strings = [Manifest.permission.READ_EXTERNAL_STORAGE], alias = "storage"),
  ],
)
class PhotosPlugin(private val activity: Activity) : Plugin(activity) {
  private val alias = if (Build.VERSION.SDK_INT >= 33) "media" else "storage"

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
    // Off the main thread: a library of tens of thousands takes a moment to
    // count, and the screen must not freeze while it does.
    Thread {
      try {
        invoke.resolve(count(filter))
      } catch (e: Exception) {
        invoke.reject(e.message ?: "could not read the photo library")
      }
    }.start()
  }

  private fun count(filter: Filter): JSObject {
    val image = MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE
    val video = MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
    val type = MediaStore.Files.FileColumns.MEDIA_TYPE
    val selection = when (filter.kind) {
      "videos" -> "$type = $video"
      "screenshots" ->
        if (Build.VERSION.SDK_INT >= 29) "$type = $image AND ${MediaStore.MediaColumns.RELATIVE_PATH} LIKE '%Screenshots%'"
        else "$type = $image AND ${MediaStore.MediaColumns.DATA} LIKE '%/Screenshots/%'"
      else -> "$type IN ($image, $video)"
    }
    val projection = mutableListOf(
      MediaStore.MediaColumns.SIZE,
      MediaStore.MediaColumns.DATE_TAKEN,
      MediaStore.MediaColumns.DATE_MODIFIED,
    )
    if (Build.VERSION.SDK_INT >= 30) projection.add(MediaStore.MediaColumns.IS_FAVORITE)

    val cutoff = if (filter.olderThanDays > 0) System.currentTimeMillis() - filter.olderThanDays * 86_400_000L else Long.MAX_VALUE
    // Sorted by year, so the screen lists newest first without sorting again.
    val years = sortedMapOf<Int, LongArray>(compareByDescending { it })
    var favCount = 0L
    var favBytes = 0L
    val calendar = Calendar.getInstance()

    activity.contentResolver.query(
      MediaStore.Files.getContentUri("external"), projection.toTypedArray(), selection, null, null,
    )?.use { rows ->
      val size = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
      val taken = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_TAKEN)
      val modified = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
      val favourite = if (Build.VERSION.SDK_INT >= 30) rows.getColumnIndexOrThrow(MediaStore.MediaColumns.IS_FAVORITE) else -1
      while (rows.moveToNext()) {
        // When the photo was taken, or failing that when the file last
        // changed (seconds, where date taken is milliseconds).
        val at = if (!rows.isNull(taken) && rows.getLong(taken) > 0) rows.getLong(taken) else rows.getLong(modified) * 1000
        if (at > cutoff) continue
        val bytes = rows.getLong(size)
        if (favourite >= 0 && rows.getInt(favourite) == 1) {
          favCount++
          favBytes += bytes
          continue
        }
        calendar.timeInMillis = at
        val tally = years.getOrPut(calendar.get(Calendar.YEAR)) { LongArray(2) }
        tally[0]++
        tally[1] += bytes
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
