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
import com.chaquo.python.PyException
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

    @Command
    fun downloadTrack(invoke: Invoke) {
        try {
            val args = invoke.getArgs()
            val query = args.getString("query")
            val outBase = args.getString("out_base")
            val song = args.getString("song")
            val artist = args.getString("artist")

            ensurePython()

            val python = Python.getInstance()
            // Get the read-only native library directory
            val nativeLibDir = activity.applicationInfo.nativeLibraryDir

            val os = python.getModule("os")
            val environ = os.get("environ")!!
            // Make sure the native dir is in PATH
            val currentPath = environ.callAttr("get", "PATH")?.toString() ?: ""
            environ.callAttr("__setitem__", "PATH", "$nativeLibDir:$currentPath")
            // Pass the directory to the Python patch script
            environ.callAttr("__setitem__", "NATIVE_LIB_DIR", nativeLibDir)

            // --- ROBUST MONKEY PATCH: SABR BYPASS & FFMPEG .SO PATH ---
            try {
                val patchScript = """
                    import os
                    import yt_dlp
                    from yt_dlp.postprocessor.ffmpeg import FFmpegPostProcessor
                    
                    # 1. SABR Bypass & 416 Prevention
                    _orig_ydl_init = yt_dlp.YoutubeDL.__init__
                    def _patched_ydl_init(self, params=None, *args, **kwargs):
                        if params is None:
                            params = {}
                        
                        # Force clients that bypass SABR and provide direct URLs
                        if 'extractor_args' not in params:
                            params['extractor_args'] = {}
                        if 'youtube' not in params['extractor_args']:
                            params['extractor_args']['youtube'] = {}
                        
                        # Use 'tv' and 'web_safari' - these are currently reliable
                        params['extractor_args']['youtube']['player_client'] = ['tv', 'web_safari']
                        
                        # Prevent resumable download errors (HTTP 416)
                        params['nopart'] = True
                        params['continuedl'] = False
                        
                        # Debug: Print the applied options to confirm patch
                        print(f"PATCHED YDL_OPTS: {params.get('extractor_args')}")
                        
                        _orig_ydl_init(self, params, *args, **kwargs)
                    
                    yt_dlp.YoutubeDL.__init__ = _patched_ydl_init
                    
                    # 2. FFmpeg .so Path Override
                    native_lib_dir = os.environ.get('NATIVE_LIB_DIR')
                    ffmpeg_path = os.path.join(native_lib_dir, 'libffmpeg.so')
                    ffprobe_path = os.path.join(native_lib_dir, 'libffprobe.so')
                    
                    _orig_ffmpeg_init = FFmpegPostProcessor.__init__
                    def _patched_ffmpeg_init(self, downloader=None):
                        _orig_ffmpeg_init(self, downloader)
                        self.executable = ffmpeg_path
                        self.probe_executable = ffprobe_path
                    FFmpegPostProcessor.__init__ = _patched_ffmpeg_init
                """.trimIndent()
                
                // Execute in the __main__ module to ensure global effect
                val mainModule = python.getModule("__main__")
                python.getModule("builtins").callAttr("exec", patchScript, mainModule.get("__dict__"))
                
            } catch (e: Exception) {
                e.printStackTrace()
            }
            // --------------------------------------------------

            val tempDir = File(activity.cacheDir, "temp_downloads")
            if (tempDir.exists()) {
                tempDir.deleteRecursively()
            }
            tempDir.mkdirs()

            val sys = python.getModule("sys")
            val io = python.getModule("io")
            
            val outCapture = io.callAttr("StringIO")
            val errCapture = io.callAttr("StringIO")
            
            val oldOut = sys.get("stdout")
            val oldErr = sys.get("stderr")
            
            sys.callAttr("__setattr__", "stdout", outCapture)
            sys.callAttr("__setattr__", "stderr", errCapture)

            var resultJson = "null"
            try {
                val module = python.getModule("downloader")
                // Pass nativeLibDir to downloader.py
                module.callAttr(
                    "download_track",
                    query,
                    tempDir.absolutePath,
                    nativeLibDir, 
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
            } catch (e: PyException) {
                val errorResponse = JSONObject()
                errorResponse.put("status", "error")
                errorResponse.put("message", "Python Crash: ${e.message}\n${e.stackTraceToString()}")
                val response = JSObject()
                response.put("result", errorResponse.toString())
                invoke.resolve(response)
                return
            } finally {
                sys.callAttr("__setattr__", "stdout", oldOut)
                sys.callAttr("__setattr__", "stderr", oldErr)
            }

            if (resultJson == "null") {
                val capturedOut = outCapture.callAttr("getvalue").toString().trim()
                val capturedErr = errCapture.callAttr("getvalue").toString().trim()
                val errorMsg = "Python output was: $capturedOut\nPython error was: $capturedErr"
                
                val errorResponse = JSONObject()
                errorResponse.put("status", "error")
                errorResponse.put("message", errorMsg)
                
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
                    
                    val response = JSObject()
                    response.put("result", resultJson)
                    invoke.resolve(response)
                    return
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