package com.d3khan.songsaver

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
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

    private val clientFallbacks = listOf("tv", "web_safari", "mweb", "ios", "android")

    private fun ensurePython() {
        if (!Python.isStarted()) {
            Python.start(AndroidPlatform(activity))
        }
    }

    private fun ensureBinary(name: String) {
        val target = File(activity.codeCacheDir, name)
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

            val ffmpegDir = activity.codeCacheDir.absolutePath
            val ffmpegPath = File(activity.codeCacheDir, "ffmpeg").absolutePath

            val python = Python.getInstance()
            val os = python.getModule("os")
            val environ = os.get("environ")!!
            val currentPath = environ.callAttr("get", "PATH")?.toString() ?: ""
            environ.callAttr("__setitem__", "PATH", "$ffmpegDir:$currentPath")

            val tempDir = File(activity.cacheDir, "temp_downloads")
            if (!tempDir.exists()) tempDir.mkdirs()

            var resultJson = "null"
            var lastError: String? = null
            var success = false

            for ((index, client) in clientFallbacks.withIndex()) {
                // Add a 3-second delay between attempts (skip for the first attempt)
                if (index > 0) {
                    try {
                        Thread.sleep(3000L) 
                    } catch (e: InterruptedException) {
                        // Handle interruption if needed
                    }
                }

                environ.callAttr("__setitem__", "YTDLP_EXTRACTOR_ARGS", "youtube:player_client=$client")

                val sys = python.getModule("sys")
                val io = python.getModule("io")
                
                val outCapture = io.callAttr("StringIO")
                val errCapture = io.callAttr("StringIO")
                
                val oldOut = sys.get("stdout")
                val oldErr = sys.get("stderr")
                
                sys.callAttr("__setattr__", "stdout", outCapture)
                sys.callAttr("__setattr__", "stderr", errCapture)

                try {
                    val module = python.getModule("downloader")
                    module.callAttr(
                        "download_track",
                        query,
                        tempDir.absolutePath,
                        ffmpegPath,
                        song,
                        artist
                    )
                    
                    val fullOutput = outCapture.callAttr("getvalue").toString().trim()
                    val lines = fullOutput.lines()
                    for (i in lines.indices.reversed()) {
                        val line = lines[i].trim()
                        if (line.startsWith("{") && line.endsWith("}")) {
                            resultJson = line
                            break
                        }
                    }
                } catch (e: Exception) {
                    lastError = e.message
                } finally {
                    sys.callAttr("__setattr__", "stdout", oldOut)
                    sys.callAttr("__setattr__", "stderr", oldErr)
                }

                if (resultJson != "null") {
                    val parsed = JSONObject(resultJson)
                    if (parsed.optString("status") == "success") {
                        success = true
                        break // Exit loop on success
                    } else {
                        lastError = parsed.optString("message", "Unknown error")
                    }
                } else {
                    lastError = "Python script produced no JSON output. Output was: ${outCapture.callAttr("getvalue").toString().trim()}"
                }
            }

            if (!success) {
                val errorResponse = JSONObject()
                errorResponse.put("status", "error")
                errorResponse.put("message", "All client attempts failed. Last error: $lastError")
                
                val response = JSObject()
                response.put("result", errorResponse.toString())
                invoke.resolve(response)
                return
            }

            val parsed = JSONObject(resultJson)
            if (parsed.optString("status") == "success") {
                val tempFilePath = parsed.getString("path")
                val tempFile = File(tempFilePath)

                val targetUri = Uri.parse(outBase)
                val fileName = tempFile.name
                val mimeType = "audio/mpeg"

                val newFileUri = DocumentsContract.createDocument(
                    activity.contentResolver, targetUri, mimeType, fileName
                )

                if (newFileUri != null) {
                    activity.contentResolver.openOutputStream(newFileUri)?.use { outputStream ->
                        tempFile.inputStream().use { inputStream ->
                            inputStream.copyTo(outputStream)
                        }
                    }
                    tempFile.delete()
                }
            }

            val response = JSObject()
            response.put("result", resultJson)
            invoke.resolve(response)

        } catch (e: Exception) {
            val errorResponse = JSONObject()
            errorResponse.put("status", "error")
            errorResponse.put("message", "Plugin Exception: ${e.message}")
            
            val response = JSObject()
            response.put("result", errorResponse.toString())
            invoke.resolve(response)
        }
    }

    @Command
    fun pickFolder(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        startActivityForResult(invoke, intent, "folderPickerResult")
    }

    @ActivityCallback
    fun folderPickerResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode == Activity.RESULT_OK) {
            val uri = result.data?.data
            if (uri != null) {
                activity.contentResolver.takePersistableUriPermission(
                    uri,
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                )
                val response = JSObject()
                response.put("path", uri.toString())
                invoke.resolve(response)
            } else {
                invoke.reject("No folder selected")
            }
        } else {
            invoke.reject("No folder selected")
        }
    }
}