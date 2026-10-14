"""Prints calliope.io.read_rich_yaml(path) in the canonical form of dump_yaml.rs.

Usage (from the Calliope pixi env): python parity_dump.py FILE...
"""

import datetime
import json
import sys

from calliope.io import read_rich_yaml


def canon(v):
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        return '{"f": "%s"}' % repr(v)
    if isinstance(v, str):
        return json.dumps(v, ensure_ascii=False)
    if isinstance(v, datetime.datetime):
        return '{"datetime": "%s"}' % v
    if isinstance(v, datetime.date):
        return '{"date": "%s"}' % v
    if isinstance(v, list):
        return "[" + ", ".join(canon(i) for i in v) + "]"
    if isinstance(v, dict):
        items = ", ".join(f"[{json.dumps(str(k), ensure_ascii=False)}, {canon(x)}]" for k, x in v.items())
        return '{"map": [' + items + "]}"
    raise TypeError(type(v))


for path in sys.argv[1:]:
    try:
        out = canon(read_rich_yaml(path))
    except Exception as e:  # noqa: BLE001
        out = '{"error": %s}' % json.dumps(f"{type(e).__name__}: {e}")
    print(f"{path}\t{out}")
