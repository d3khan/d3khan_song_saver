package com.d3khan.songsaver

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.DocumentsContract
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import app.tauri.plugin.JSObject
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import java.io.File
import org.json.JSONObject

@TauriPlugin
class DownloaderPlugin(private val activity: Activity) : Plugin(activity) {

    private fun ensurePython() {
        if (!Python.isStarted()) {
            Python.start(AndroidPlatform(activity))
        }
    }

    private fun ensureBinary(name: String) {
        val target = File(activity.filesDir, name)
        if (!target.exists() || target.length() == 0L) {
            activity.assets.open(name).use { input ->
                target.outputStream().use { output ->
                    input.copyTo(output)
                }
            }
        }
        target.setReadable(true, false)
        target.setWritable(true, false)
        target.setExecutable(true, false)
    }

    @Command
    fun downloadTrack(invoke: Invoke) {
        try {
            val args = invoke.getArgs()
            val query = args.getString("query")
            val outBase = args.getString("out_base")
            val song = args.getString("song")
            val artist = args.getString("artist")

            ensurePython()
            ensureBinary("ffmpeg")
            ensureBinary("ffprobe")

            val ffmpegDir = activity.filesDir.absolutePath
            val ffmpegPath = File(activity.filesDir, "ffmpeg").absolutePath

            val os = Python.getInstance().getModule("os")
            val environ = os.get("environ")!!
            val currentPath = environ.callAttr("get", "PATH")?.toString() ?: ""
            environ.callAttr("__setitem__", "PATH", "$ffmpegDir:$currentPath")

            val module = Python.getInstance().getModule("downloader")
            val resultJson = module.callAttr(
                "download_track",
                query,
                outBase,
                ffmpegPath,
                song,
                artist
            )?.toString() ?: "null"

            val response = JSObject()
            response.put("result", resultJson)
            invoke.resolve(response)

        } catch (e: Exception) {
            val errorResponse = JSONObject()
            errorResponse.put("status", "error")
            errorResponse.put("message", "Python Error: ${e.message}")
            
            val response = JSObject()
            response.put("result", errorResponse.toString())
            invoke.resolve(response)
        }
    }

    @Command
    fun pickFolder(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        // Tauri's startActivityForResult handles mapping the invoke automatically
        startActivityForResult(invoke, intent, 1234)
    }

    override fun onActivityResult(invoke: Invoke, requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(invoke, requestCode, resultCode, data)
        if (requestCode == 1234) {
            if (resultCode == Activity.RESULT_OK && data != null) {
                val uri = data.data
                if (uri != null) {
                    val path = getPathFromUri(uri)
                    if (path != null) {
                        val response = JSObject()
                        response.put("path", path)
                        invoke.resolve(response)
                    } else {
                        invoke.reject("Could not resolve folder path")
                    }
                } else {
                    invoke.reject("No folder selected")
                }
            } else {
                invoke.reject("No folder selected")
            }
        }
    }

    private fun getPathFromUri(uri: Uri): String? {
        try {
            val docId = DocumentsContract.getTreeDocumentId(uri)
            val split = docId.split(":")
            val type = split[0]
            var path = ""
            if ("primary".equals(type, ignoreCase = true)) {
                path = Environment.getExternalStorageDirectory().toString() + "/" + split[1]
            } else {
                val sdCardPath = System.getenv("SECONDARY_STORAGE")
                if (sdCardPath != null) {
                    path = sdCardPath + "/" + split[1]
                }
            }
            return path
        } catch (e: Exception) {
            return null
        }
    }
}