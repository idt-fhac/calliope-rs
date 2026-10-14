"""Stage timing and peak-RSS instrumentation for Python Calliope runs.

Stages are measured from the outside (wall clock around the public API calls) and,
for the solve, split from the inside by wrapping backend methods:

- ``preprocess``: ``calliope.read_yaml`` (model definition -> inputs dataset)
- ``build``: ``Model.build``
- ``solve``: ``Model.solve`` minus time in ``load_results``
- ``postprocess``: time in ``BackendModel.load_results`` (results + postprocessed math)
- ``write``: ``Model.to_netcdf``

Peak RSS per stage comes from a sampler thread reading ``/proc/self/statm``.
Per-iteration events (operate windows, SPORES iterations, ``update_input`` calls,
rebuilds) are recorded by wrapping backend methods.
"""

from __future__ import annotations

import contextlib
import functools
import os
import resource
import threading
import time
from dataclasses import dataclass, field

_PAGE = os.sysconf("SC_PAGE_SIZE")


def current_rss() -> int:
    with open("/proc/self/statm") as f:
        return int(f.read().split()[1]) * _PAGE


class RssSampler:
    """Background thread tracking the max RSS since the last ``reset``."""

    def __init__(self, interval: float = 0.005):
        self.interval = interval
        self._peak = 0
        self._stop = threading.Event()
        self._lock = threading.Lock()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self):
        while not self._stop.is_set():
            rss = current_rss()
            with self._lock:
                if rss > self._peak:
                    self._peak = rss
            time.sleep(self.interval)

    def reset(self) -> int:
        """Return the peak since the last reset and start a new window."""
        rss = current_rss()
        with self._lock:
            peak = max(self._peak, rss)
            self._peak = rss
        return peak

    def stop(self):
        self._stop.set()
        self._thread.join()


@dataclass
class Recorder:
    stages: dict = field(default_factory=dict)
    events: list = field(default_factory=list)
    sampler: RssSampler = field(default_factory=RssSampler)
    _t0: float = field(default_factory=time.perf_counter)

    @contextlib.contextmanager
    def stage(self, name: str):
        self.sampler.reset()
        start_rss = current_rss()
        t = time.perf_counter()
        try:
            yield
        finally:
            dt = time.perf_counter() - t
            peak = self.sampler.reset()
            entry = self.stages.setdefault(
                name, {"wall_s": 0.0, "peak_rss_mb": 0.0, "start_rss_mb": None}
            )
            entry["wall_s"] += dt
            entry["peak_rss_mb"] = max(entry["peak_rss_mb"], peak / 2**20)
            if entry["start_rss_mb"] is None:
                entry["start_rss_mb"] = start_rss / 2**20

    def event(self, kind: str, dt: float, **info):
        self.events.append(
            {
                "kind": kind,
                "t": round(time.perf_counter() - self._t0, 6),
                "wall_s": dt,
                "rss_mb": current_rss() / 2**20,
                **info,
            }
        )

    def summary(self) -> dict:
        # postprocess is nested inside solve: report solve exclusive of it
        stages = {k: dict(v) for k, v in self.stages.items()}
        pp = sum(e["wall_s"] for e in self.events if e["kind"] == "load_results")
        if "solve" in stages:
            stages["solve"]["wall_s_incl_postprocess"] = stages["solve"]["wall_s"]
            stages["solve"]["wall_s"] -= pp
            stages["postprocess"] = {"wall_s": pp, "nested_in": "solve"}
        total = sum(v["wall_s"] for k, v in stages.items() if k != "postprocess") + pp
        for v in stages.values():
            v["share"] = v["wall_s"] / total if total else None
        return {
            "stages": stages,
            "total_wall_s": total,
            "process_peak_rss_mb": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
            / 1024,
            "events": self.events,
        }


def _wrap(cls, name: str, rec: Recorder, kind: str, info=None):
    orig = getattr(cls, name)

    @functools.wraps(orig)
    def wrapper(*args, **kwargs):
        t = time.perf_counter()
        try:
            return orig(*args, **kwargs)
        finally:
            extra = info(*args, **kwargs) if info else {}
            rec.event(kind, time.perf_counter() - t, **extra)

    setattr(cls, name, wrapper)
    return orig


def install(rec: Recorder) -> list:
    """Monkeypatch Calliope classes so that ``rec`` gets per-call events."""
    from calliope import model as model_mod
    from calliope.backend import backend_model, highs_backend_model, pyomo_backend_model

    patched = []
    patched.append(
        (
            backend_model.BackendModel,
            "load_results",
            _wrap(backend_model.BackendModel, "load_results", rec, "load_results"),
        )
    )
    for cls in (
        highs_backend_model.HighsBackendModel,
        pyomo_backend_model.PyomoBackendModel,
    ):
        patched.append((cls, "_solve", _wrap(cls, "_solve", rec, "backend_solve")))
        patched.append(
            (
                cls,
                "update_input",
                _wrap(
                    cls,
                    "update_input",
                    rec,
                    "update_input",
                    info=lambda self, name, *a, **k: {"name": name},
                ),
            )
        )
    patched.append(
        (
            model_mod.Model,
            "build",
            _wrap(model_mod.Model, "build", rec, "model_build"),
        )
    )
    return patched


def uninstall(patched: list):
    for cls, name, orig in reversed(patched):
        setattr(cls, name, orig)
