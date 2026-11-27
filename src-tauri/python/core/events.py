"""Single JSON protocol out of the sidecar. Rust reads stdout lines."""
import json
import sys


def emit(obj):
    print(json.dumps(obj), flush=True)


def success(**kw):
    emit({"status": "success", **kw})


def fail(msg):
    emit({"status": "error", "message": str(msg)})


def warn(msg):
    print(f"WARNING: {msg}", file=sys.stderr, flush=True)


def progress_hook(d):
    if d.get("status") == "downloading":
        done = d.get("downloaded_bytes") or 0
        total = d.get("total_bytes") or d.get("total_bytes_estimate") or 0
        emit({
            "event": "progress",
            "percent": round(done * 100.0 / total, 1) if total else 0,
            "downloaded": done,
            "total": total,
            "speed": d.get("speed") or 0,
            "eta": d.get("eta") or 0,
        })
    elif d.get("status") == "finished":
        emit({"event": "stage", "message": "Download complete, processing..."})


def pp_hook(d):
    if d.get("status") == "started":
        emit({"event": "stage", "message": f"Running {d.get('postprocessor')}..."})
