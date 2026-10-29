package com.d3khan.songsaver

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import app.tauri.plugin.JSObject
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import java.io.File

@TauriPlugin
class DownloaderPlugin(private val activity: Activity) : Plugin(activity) {

    private fun ensurePython() {
        if (!Python.isStarted()) Python.start(AndroidPlatform(activity))
    }

    private fun ensureBinary(name: String) {
        val target = File(activity.filesDir, name)
        if (!target.exists() || target.length() == 0L) {
            activity.assets.open(name).use { input ->
                target.outputStream().use { output -> input.copyTo(output) }
            }
            target.setExecutable(true)
        }
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
            
            // Kotlin constructs the true Android path for FFmpeg
            val ffmpegPath = File(activity.filesDir, "ffmpeg").absolutePath

            val module = Python.getInstance().getModule("downloader")
            val result = module.callAttr("download_track", query, outBase, ffmpegPath, song, artist).toString()
            
            // Tauri v2 requires a JSObject to resolve the promise
            val response = JSObject()
            response.put("result", result)
            invoke.resolve(response)
            
        } catch (e: Exception) {
            invoke.reject("Python error: ${e.message}")
        }
    }
}