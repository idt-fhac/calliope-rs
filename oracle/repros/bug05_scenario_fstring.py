"""#5 model_definition.py:176: error message lacks the ``f`` prefix.

A scenario whose list contains a dict raises a ModelError that prints
``{scenario}`` literally.
"""

import calliope
from _common import quiet, result
from calliope.io import read_rich_yaml

quiet()
model_def = read_rich_yaml(
    """
config.init.name: repro
scenarios:
  bad_scenario: [{foo: 1}]
overrides:
  foo: {}
nodes: {}
techs: {}
"""
)
try:
    calliope.read_dict(model_def, scenario="bad_scenario")
except calliope.exceptions.ModelError as err:
    msg = str(err)
    result(
        "confirmed" if "{scenario}" in msg else "not a bug",
        f"message={msg!r}",
    )
except Exception as err:  # noqa: BLE001
    result("inconclusive", f"{type(err).__name__}: {err}")
else:
    result("not a bug", "no error raised")
