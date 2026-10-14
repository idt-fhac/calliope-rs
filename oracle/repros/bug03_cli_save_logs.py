"""#3 CLI ``--save_logs`` and ``save_per_spore_path`` handling.

cli.py:284 passes ``{"solve.save_logs": path}`` as ``solve(**kwargs)``; ``Model.solve``
then does ``config.update({"solve": kwargs})``, i.e. ``solve.solve.save_logs``.
cli.py:300 passes ``save_per_spore_path`` as a 1-tuple ``(Path,)``.
"""

import tempfile
from pathlib import Path

import calliope
from _common import NATIONAL, TEST_MODEL, quiet, result
from calliope import cli
from click.testing import CliRunner

quiet()
tmp = Path(tempfile.mkdtemp())
logdir = tmp / "logs"
runner = CliRunner()
res = runner.invoke(
    cli.run,
    [
        str(TEST_MODEL),
        "--scenario=simple_supply,two_hours,investment_costs",
        "--override_dict={config.build.backend: highs}",
        f"--save_logs={logdir}",
        f"--save_netcdf={tmp / 'out.nc'}",
    ],
)
logs = sorted(p.name for p in logdir.rglob("*")) if logdir.exists() else []
m = calliope.read_netcdf(tmp / "out.nc")
saved = m.config.solve.save_logs
detail_logs = (
    f"exit={res.exit_code}; files in --save_logs dir: {logs or 'none'}; "
    f"config.solve.save_logs in output: {saved!r}"
)

# Direct API check of what solve(**{"solve.save_logs": x}) does to the config
m2 = calliope.read_yaml(
    TEST_MODEL,
    scenario="simple_supply,two_hours,investment_costs",
    override_dict={"config.build.backend": "highs"},
)
m2.build()
m2.solve(
    **{"solve.save_logs": str(logdir / "api")}, solver_options={"output_flag": False}
)
detail_api = (
    f"Model.solve(**{{'solve.save_logs': ...}}) -> config.solve.save_logs="
    f"{m2.config.solve.save_logs!r}, dir exists={Path(logdir / 'api').exists()}"
)

# save_per_spore_path with --save_netcdf in a different directory than the override
sp_dir = tmp / "per_spore_from_override"
nc_dir = tmp / "nc_dir"
nc_dir.mkdir()
res2 = runner.invoke(
    cli.run,
    [
        str(NATIONAL),
        "--scenario=spores",
        "--override_dict={config.build.backend: highs, config.solve.spores.number: 1, "
        f"config.solve.spores.save_per_spore_path: '{sp_dir}', "
        "config.init.subset.timesteps: ['2005-01-01', '2005-01-01']}",
        f"--save_netcdf={nc_dir / 'out.nc'}",
    ],
)
spore_files = sorted(p.relative_to(tmp).as_posix() for p in tmp.rglob("spore_*.nc"))
detail_spores = f"spores exit={res2.exit_code}; per-spore files: {spore_files}"
if res2.exit_code != 0:
    detail_spores += f"; output tail: {res2.output[-300:]!r}"

bug_logs = not logs
result(
    "confirmed" if bug_logs else "not a bug",
    f"{detail_logs}. {detail_api}. {detail_spores}",
)
