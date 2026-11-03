package com.ifeanyiibekie.tungstate.photos

import android.Manifest
import android.app.Activity
import android.content.ContentUris
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import android.provider.MediaStore
import android.view.WindowManager
import androidx.activity.result.ActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import android.webkit.MimeTypeMap
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
class CategoryArgs {
  /** "images", "videos", "audio", "documents", "downloads" or "apps". */
  var category: String = "images"
}

@InvokeArg
class PathsArgs {
  var paths: Array<String> = emptyArray()
}

@InvokeArg
class AwakeArgs {
  var on: Boolean = false
}

/** One file in a category, by its real path. */
private class Stored(val path: String, val name: String, val size: Long, val modified: Long)

/** Endings that make a file a document when the index gave no type. */
private val DOCUMENT_ENDINGS = listOf(".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".odt", ".ods", ".txt", ".csv", ".rtf", ".epub", ".md")

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
    // Android 10 and earlier: a file manager reads and writes shared storage
    // with these. 11 and later use All files access instead, a settings page
    // rather than a prompt, which is why it has no alias here.
    Permission(
      strings = [Manifest.permission.READ_EXTERNAL_STORAGE, Manifest.permission.WRITE_EXTERNAL_STORAGE],
      alias = "files",
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

  /** "full" once the app may read and change all of shared storage, else
   *  "ask"; Android never says "denied" for All files access. */
  private fun filesState(): String =
    if (Build.VERSION.SDK_INT >= 30) {
      if (Environment.isExternalStorageManager()) "full" else "ask"
    } else if (granted(Manifest.permission.WRITE_EXTERNAL_STORAGE)) {
      "full"
    } else if (getPermissionState("files") == PermissionState.DENIED) {
      "denied"
    } else {
      "ask"
    }

  private fun answerFiles(invoke: Invoke) {
    val ret = JSObject()
    ret.put("access", filesState())
    invoke.resolve(ret)
  }

  @Command
  fun filesAccess(invoke: Invoke) = answerFiles(invoke)

  /** Opens Android's own All files access page for this app, and answers
   *  once the person comes back from it. */
  @Command
  fun askFiles(invoke: Invoke) {
    if (filesState() == "full") return answerFiles(invoke)
    if (Build.VERSION.SDK_INT < 30) return requestPermissionForAlias("files", invoke, "filesAsked")
    val mine = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:${activity.packageName}"))
    // Some phones lack the per-app page; the list of apps is the next best.
    val intent = if (mine.resolveActivity(activity.packageManager) != null) mine else Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
    startActivityForResult(invoke, intent, "filesBack")
  }

  @PermissionCallback
  private fun filesAsked(invoke: Invoke) = answerFiles(invoke)

  // The page returns no result of its own; what matters is the state after.
  @ActivityCallback
  private fun filesBack(invoke: Invoke, @Suppress("UNUSED_PARAMETER") result: ActivityResult) = answerFiles(invoke)

  /** Counts and bytes for each category tile, in one pass over the index. */
  @Command
  fun categories(invoke: Invoke) {
    offMain(invoke, "could not count what is on the phone") {
      val names = listOf("images", "videos", "audio", "documents", "downloads", "apps")
      val tally = names.associateWith { LongArray(2) }
      files { file, kinds ->
        for (kind in kinds) {
          val t = tally.getValue(kind)
          t[0]++
          t[1] += file.size
        }
      }
      val ret = JSObject()
      for ((name, t) in tally) {
        val one = JSObject()
        one.put("count", t[0])
        one.put("bytes", t[1])
        ret.put(name, one)
      }
      invoke.resolve(ret)
    }
  }

  /** Every file in one category, newest first, by its real path. */
  @Command
  fun categoryFiles(invoke: Invoke) {
    val args = invoke.parseArgs(CategoryArgs::class.java)
    offMain(invoke, "could not list what is on the phone") {
      val found = mutableListOf<Stored>()
      files { file, kinds -> if (args.category in kinds) found.add(file) }
      found.sortByDescending { it.modified }
      val list = JSArray()
      for (file in found) {
        val one = JSObject()
        one.put("path", file.path)
        one.put("name", file.name)
        one.put("size", file.size)
        one.put("modifiedMs", file.modified)
        list.put(one)
      }
      val ret = JSObject()
      ret.put("files", list)
      invoke.resolve(ret)
    }
  }

  /**
   * Every file the system's index knows, with the categories it is in.
   *
   * The index, not a walk: Android keeps it current for all of shared
   * storage, and walking a phone's 100k files would take seconds. With All
   * files access it lists every file, not only photos, video and audio.
   * `DATA` is the real path; deprecated for reading files, but still filled
   * in, and the engine needs a path rather than a content address.
   */
  private fun files(visit: (Stored, List<String>) -> Unit) {
    val type = MediaStore.Files.FileColumns.MEDIA_TYPE
    @Suppress("DEPRECATION")
    val data = MediaStore.MediaColumns.DATA
    val projection = arrayOf(
      data, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.SIZE,
      MediaStore.MediaColumns.DATE_MODIFIED, MediaStore.MediaColumns.MIME_TYPE, type,
    )
    val downloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS).path + "/"
    activity.contentResolver.query(MediaStore.Files.getContentUri("external"), projection, null, null, null)?.use { rows ->
      val pathAt = rows.getColumnIndexOrThrow(data)
      val nameAt = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
      val sizeAt = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
      val modifiedAt = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
      val mimeAt = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.MIME_TYPE)
      val typeAt = rows.getColumnIndexOrThrow(type)
      while (rows.moveToNext()) {
        val path = rows.getString(pathAt) ?: continue
        // Folders are in the index too, with no type; a tile counts files.
        if (rows.isNull(mimeAt) && File(path).isDirectory) continue
        // A file copied in moments ago is listed before its size is filled
        // in; the file itself knows, and All files access may ask it.
        val size = if (rows.isNull(sizeAt)) File(path).length() else rows.getLong(sizeAt)
        val name = rows.getString(nameAt) ?: path.substringAfterLast('/')
        val mime = rows.getString(mimeAt) ?: ""
        val kinds = mutableListOf<String>()
        when (rows.getInt(typeAt)) {
          MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE -> kinds.add("images")
          MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO -> kinds.add("videos")
          MediaStore.Files.FileColumns.MEDIA_TYPE_AUDIO -> kinds.add("audio")
          else -> when {
            mime == "application/vnd.android.package-archive" || name.endsWith(".apk", true) -> kinds.add("apps")
            document(mime, name) -> kinds.add("documents")
          }
        }
        if (path.startsWith(downloads)) kinds.add("downloads")
        if (kinds.isEmpty()) continue
        val modified = if (rows.isNull(modifiedAt)) File(path).lastModified() else rows.getLong(modifiedAt) * 1000
        visit(Stored(path, name, size, modified), kinds)
      }
    }
  }

  private fun document(mime: String, name: String): Boolean =
    mime == "application/pdf" || mime.startsWith("text/") || mime == "application/rtf" ||
      mime == "application/epub+zip" || mime.startsWith("application/msword") ||
      mime.startsWith("application/vnd.ms-") || mime.startsWith("application/vnd.openxmlformats-officedocument") ||
      mime.startsWith("application/vnd.oasis.opendocument") ||
      DOCUMENT_ENDINGS.any { name.endsWith(it, true) }

  /** A content address another app may read, through the app's own
   *  FileProvider; it refuses anything outside the paths it was given. */
  private fun shareable(path: String): Uri =
    FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", File(path))

  private fun mimeOf(path: String): String =
    MimeTypeMap.getSingleton().getMimeTypeFromExtension(path.substringAfterLast('.', "").lowercase()) ?: "*/*"

  @Command
  fun share(invoke: Invoke) {
    val args = invoke.parseArgs(PathsArgs::class.java)
    try {
      val uris = ArrayList(args.paths.map(::shareable))
      val types = args.paths.map(::mimeOf).distinct()
      val intent = Intent(if (uris.size == 1) Intent.ACTION_SEND else Intent.ACTION_SEND_MULTIPLE).apply {
        type = types.singleOrNull() ?: "*/*"
        if (uris.size == 1) putExtra(Intent.EXTRA_STREAM, uris[0]) else putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      activity.startActivity(Intent.createChooser(intent, null))
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message ?: "could not share these")
    }
  }

  @Command
  fun open(invoke: Invoke) {
    val args = invoke.parseArgs(PathsArgs::class.java)
    try {
      val path = args.paths.first()
      val intent = Intent(Intent.ACTION_VIEW).apply {
        setDataAndType(shareable(path), mimeOf(path))
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      activity.startActivity(intent)
      invoke.resolve()
    } catch (e: android.content.ActivityNotFoundException) {
      invoke.reject("no app on this phone opens this kind of file")
    } catch (e: Exception) {
      invoke.reject(e.message ?: "could not open it")
    }
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
