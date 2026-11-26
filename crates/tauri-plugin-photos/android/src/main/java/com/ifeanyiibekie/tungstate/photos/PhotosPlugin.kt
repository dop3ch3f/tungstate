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
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.media.MediaMetadataRetriever
import android.media.ThumbnailUtils
import android.net.wifi.WifiManager
import android.util.Size
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
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
  /** Only this many, picked in `order`; 0 for every one. */
  var first: Int = 0
  /** "newest", "oldest", "name" or "largest", as the window sorts. */
  var order: String = "newest"
}

@InvokeArg
class PathsArgs {
  var paths: Array<String> = emptyArray()
}

@InvokeArg
class AwakeArgs {
  var on: Boolean = false
}

@InvokeArg
class NoticeArgs {
  var title: String = ""
  var text: String = ""
  var permille: Int = 0
  var waiting: Boolean = false

  fun notice() = Notice(title, text, permille, waiting)
}

@InvokeArg
class ShrinkArgs {
  lateinit var path: String
  var size: Int = 256
  lateinit var out: String
}

/** A filter for the system's index: a `WHERE` clause and its `?` values. */
private class Narrow(val where: String, val args: List<String> = emptyList())

/** One file in a category, by its real path. */
private class Stored(val path: String, val name: String, val size: Long, val modified: Long)

/** The window's sorts as the index's `ORDER BY`. */
private val ORDERS = mapOf(
  "newest" to "${MediaStore.MediaColumns.DATE_MODIFIED} DESC",
  "oldest" to "${MediaStore.MediaColumns.DATE_MODIFIED} ASC",
  "name" to "${MediaStore.MediaColumns.DISPLAY_NAME} COLLATE NOCASE ASC",
  "largest" to "${MediaStore.MediaColumns.SIZE} DESC",
)

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

  override fun load(webView: android.webkit.WebView) {
    super.load(webView)
    TransferKeeper.clearStale(activity)
  }

  // Commands arrive on the main thread. Reading the library and copying out
  // of it can take a while, so it happens here, one job at a time.
  private val worker = Executors.newSingleThreadExecutor()

  // Pictures for file lists come in bursts as a folder scrolls past; four at
  // a time keeps the phone's decoders busy without starving everything else,
  // and they never queue behind a long photo export on `worker`.
  private val pictures = Executors.newFixedThreadPool(4)

  // Category lists are what a person is waiting on with an empty screen, so
  // they get their own lane rather than queueing behind the tiles' tally or
  // a Free up count on `worker`.
  private val lists = Executors.newSingleThreadExecutor()

  private var multicastLock: WifiManager.MulticastLock? = null

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
  private fun offMain(invoke: Invoke, fallback: String, on: java.util.concurrent.Executor = worker, job: () -> Unit) {
    on.execute {
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
      files(categoryFilters.values) { file, kinds ->
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

  /**
   * Every file in one category, newest first, by its real path; or only the
   * `first` few in `order`. Downloads can hold 100k files and take seconds to
   * list in full, while its first screenful comes back in about one, so the
   * window asks for that first and draws it while the rest comes.
   */
  @Command
  fun categoryFiles(invoke: Invoke) {
    val args = invoke.parseArgs(CategoryArgs::class.java)
    val narrow = categoryFilters[args.category] ?: return invoke.reject("there is no category called ${args.category}")
    offMain(invoke, "could not list what is on the phone", lists) {
      val found = mutableListOf<Stored>()
      if (args.first > 0) {
        val order = ORDERS[args.order] ?: ORDERS.getValue("newest")
        files(listOf(narrow), order, args.first) { file, kinds -> if (args.category in kinds) found.add(file) }
      } else {
        files(listOf(narrow)) { file, kinds -> if (args.category in kinds) found.add(file) }
        found.sortByDescending { it.modified }
      }
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
   * Each category as a filter the index applies itself, so a list reads
   * thousands of rows rather than all of a phone's quarter-million (every
   * folder is a row too). [files] still decides each row in code; this only
   * narrows. It filters by type rather than by name: Android types every file
   * by its ending when it indexes it, and a `LIKE` on paths costs the index
   * about a second per pattern on a full phone. Rows with no type are let
   * through, so the ending checks in code still see them.
   */
  private val categoryFilters: Map<String, Narrow> by lazy {
    val type = MediaStore.Files.FileColumns.MEDIA_TYPE
    val mime = MediaStore.MediaColumns.MIME_TYPE
    @Suppress("DEPRECATION")
    val data = MediaStore.MediaColumns.DATA
    val other = "$type NOT IN (1, 2, 3) AND ($mime IS NULL OR "
    val docs = listOf("$mime = 'application/pdf'", "$mime LIKE 'text/%'", "$mime = 'application/rtf'",
      "$mime = 'application/epub+zip'", "$mime LIKE 'application/msword%'", "$mime LIKE 'application/vnd.ms-%'",
      "$mime LIKE 'application/vnd.openxmlformats-officedocument%'", "$mime LIKE 'application/vnd.oasis.opendocument%'")
    val downloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS).path + "/"
    mapOf(
      "images" to Narrow("$type = 1"),
      "audio" to Narrow("$type = 2"),
      "videos" to Narrow("$type = 3"),
      "apps" to Narrow("$other$mime = 'application/vnd.android.package-archive')"),
      "documents" to Narrow("$other${docs.joinToString(" OR ")})"),
      // One prefix pattern is cheap; it is the dozen endings that were not.
      "downloads" to Narrow("$data LIKE ?", listOf("$downloads%")),
    )
  }

  /**
   * Every file the system's index knows that matches any of `narrow`, with
   * the categories it is in.
   *
   * The index, not a walk: Android keeps it current for all of shared
   * storage, and walking a phone's 100k files would take seconds. With All
   * files access it lists every file, not only photos, video and audio.
   * `DATA` is the real path; deprecated for reading files, but still filled
   * in, and the engine needs a path rather than a content address.
   */
  private fun files(narrow: Collection<Narrow>, order: String? = null, limit: Int = 0, visit: (Stored, List<String>) -> Unit) {
    // `format` 12289 is MTP's "association": a folder. Leaving them out here
    // spares a disk check per folder below; one the index left untyped still
    // gets it. The column is real but its constant is hidden from apps.
    val where = "(${narrow.joinToString(" OR ") { "(${it.where})" }}) AND IFNULL(format, 0) != 12289"
    val args = narrow.flatMap { it.args }.toTypedArray()
    val type = MediaStore.Files.FileColumns.MEDIA_TYPE
    @Suppress("DEPRECATION")
    val data = MediaStore.MediaColumns.DATA
    val projection = arrayOf(
      data, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.SIZE,
      MediaStore.MediaColumns.DATE_MODIFIED, MediaStore.MediaColumns.MIME_TYPE, type,
    )
    val downloads = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS).path + "/"
    var seen = 0
    activity.contentResolver.query(MediaStore.Files.getContentUri("external"), projection, where, args, order)?.use { rows ->
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
        // Stopping here rather than with the index's own LIMIT: that counts
        // rows before the checks above, so a short answer could not be told
        // from a complete one. Rows cross over only as they are read, so the
        // rest are never sent.
        if (limit > 0 && ++seen >= limit) break
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

  /** Holds multicast open while the app looks for servers: Android drops mDNS
   *  answers addressed to other apps otherwise. Released when the look ends. */
  @Command
  fun multicast(invoke: Invoke) {
    val args = invoke.parseArgs(AwakeArgs::class.java)
    try {
      if (args.on && multicastLock == null) {
        val wifi = activity.applicationContext.getSystemService(android.content.Context.WIFI_SERVICE) as WifiManager
        multicastLock = wifi.createMulticastLock("tungstate-discovery").apply {
          setReferenceCounted(false)
          acquire()
        }
      } else if (!args.on) {
        multicastLock?.release()
        multicastLock = null
      }
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message ?: "could not listen for servers")
    }
  }

  /** The status bar's and the navigation bar's heights, in CSS pixels. */
  @Command
  fun insets(invoke: Invoke) {
    activity.runOnUiThread {
      val ret = JSObject()
      val density = activity.resources.displayMetrics.density
      val insets = ViewCompat.getRootWindowInsets(activity.window.decorView)
        ?.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      ret.put("top", (insets?.top ?: 0) / density)
      ret.put("bottom", (insets?.bottom ?: 0) / density)
      invoke.resolve(ret)
    }
  }

  /**
   * A JPEG of one file, no bigger than `size` on its long side, written to
   * `out`. The phone's own decoders do the work, so HEIC photos, video
   * frames, app installer icons and song covers all come out, rotated the way
   * the camera meant.
   */
  @Command
  fun thumbnail(invoke: Invoke) {
    val args = invoke.parseArgs(ShrinkArgs::class.java)
    pictures.execute {
      try {
        val picture = shrink(File(args.path), args.size) ?: return@execute invoke.reject("no picture for this kind of file")
        File(args.out).outputStream().use { picture.compress(Bitmap.CompressFormat.JPEG, 82, it) }
        picture.recycle()
        invoke.resolve()
      } catch (e: Exception) {
        invoke.reject(e.message ?: "could not make a picture")
      }
    }
  }

  private fun shrink(file: File, size: Int): Bitmap? {
    val box = Size(size, size)
    // Free up lists photos by the library's own address rather than a path.
    if (file.path.startsWith("content:")) {
      if (Build.VERSION.SDK_INT < 29) return null
      return activity.contentResolver.loadThumbnail(Uri.parse(file.path.replaceFirst("content:/", "content://")), box, null)
    }
    val mime = mimeOf(file.path)
    return when {
      mime.startsWith("image/") ->
        if (Build.VERSION.SDK_INT >= 29) ThumbnailUtils.createImageThumbnail(file, box, null) else sampled(file, size)
      mime.startsWith("video/") ->
        if (Build.VERSION.SDK_INT >= 29) ThumbnailUtils.createVideoThumbnail(file, box, null) else frame(file)
      mime.startsWith("audio/") -> cover(file)
      mime == "application/vnd.android.package-archive" -> icon(file, size)
      else -> null
    }
  }

  /** Android 9 and earlier: decode at the nearest power of two above `size`. */
  private fun sampled(file: File, size: Int): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.path, bounds)
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= size && bounds.outHeight / (sample * 2) >= size) sample *= 2
    return BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })
  }

  private fun frame(file: File): Bitmap? = MediaMetadataRetriever().run {
    try {
      setDataSource(file.path)
      frameAtTime
    } finally {
      release()
    }
  }

  private fun cover(file: File): Bitmap? = MediaMetadataRetriever().run {
    try {
      setDataSource(file.path)
      embeddedPicture?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
    } finally {
      release()
    }
  }

  /** An installer's icon. Android reads it from the archive only once told
   *  where the archive is, hence the two source paths. */
  private fun icon(file: File, size: Int): Bitmap? {
    val pm = activity.packageManager
    val info = pm.getPackageArchiveInfo(file.path, 0)?.applicationInfo ?: return null
    info.sourceDir = file.path
    info.publicSourceDir = file.path
    val drawable = info.loadIcon(pm)
    val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
    // White behind it, since JPEG has no transparency and black would swallow dark icons.
    bitmap.eraseColor(android.graphics.Color.WHITE)
    drawable.setBounds(0, 0, size, size)
    drawable.draw(Canvas(bitmap))
    return bitmap
  }

  /** A transfer the person started has begun: keep it going with the screen
   *  off or the app in the background (TransferKeeper), and show it. */
  @Command
  fun runBegan(invoke: Invoke) {
    val args = invoke.parseArgs(NoticeArgs::class.java)
    try {
      TransferKeeper.begin(activity, args.notice())
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message ?: "could not keep the transfer going")
    }
  }

  /** Shows how far, and answers `stop: true` if Stop was pressed there. */
  @Command
  fun runProgress(invoke: Invoke) {
    val args = invoke.parseArgs(NoticeArgs::class.java)
    val ret = JSObject()
    ret.put("stop", TransferKeeper.update(activity, args.notice()))
    invoke.resolve(ret)
  }

  @Command
  fun runEnded(invoke: Invoke) {
    TransferKeeper.end(activity)
    invoke.resolve()
  }

  /** Asks to show notifications (13 and later), once, at the first transfer.
   *  Answers at once: the transfer does not wait on the person's choice. */
  @Command
  fun askNotifications(invoke: Invoke) {
    if (Build.VERSION.SDK_INT >= 33 && !granted(Manifest.permission.POST_NOTIFICATIONS)) {
      val prefs = activity.getSharedPreferences("tungstate-photos", android.content.Context.MODE_PRIVATE)
      if (!prefs.getBoolean("asked-notifications", false)) {
        prefs.edit().putBoolean("asked-notifications", true).apply()
        activity.runOnUiThread {
          androidx.core.app.ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 4711)
        }
      }
    }
    invoke.resolve()
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
