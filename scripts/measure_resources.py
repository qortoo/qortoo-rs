#!/usr/bin/env python3
"""Measure isolated SDK processes on macOS/Linux; emit raw JSON, never a CI gate."""
import argparse
import datetime
import hashlib
import json
import pathlib
import platform
import selectors
import subprocess
import time


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def snapshot(pid):
    rss = int(command("ps", "-o", "rss=", "-p", str(pid)))
    if platform.system() == "Darwin":
        # macOS ps -M emits one header followed by one row per thread.
        threads = len(command("ps", "-M", "-p", str(pid)).splitlines()) - 1
    else:
        threads = int(command("ps", "-o", "nlwp=", "-p", str(pid)))
    return {"threads": threads, "rss_kib": rss}


def probe(binary, clients, datatypes, iterations, backend, cleanup, settle, timeout, retention_wait):
    stages = []
    with subprocess.Popen(
        [str(binary), str(clients), str(datatypes), str(iterations), backend, cleanup],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    ) as child:
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            try:
                while True:
                    if not selector.select(timeout):
                        raise TimeoutError(f"probe stalled: {clients} clients, {datatypes} datatypes")
                    line = child.stdout.readline()
                    if not line:
                        break
                    stage = json.loads(line)
                    received_at = time.monotonic()
                    time.sleep(settle)
                    # Keep all samples: stable thread counts do not imply allocator RSS is flat.
                    stage["resources"] = []
                    for _ in range(3):
                        sample = snapshot(child.pid)
                        sample["seconds_after_checkpoint"] = time.monotonic() - received_at
                        stage["resources"].append(sample)
                        time.sleep(0.05)
                    if stage["stage"] == "released" and backend == "local" and cleanup == "drop":
                        # Distinguish Tokio's idle blocking-thread cache from retained workers.
                        time.sleep(retention_wait)
                        sample = snapshot(child.pid)
                        sample["seconds_after_checkpoint"] = time.monotonic() - received_at
                        stage["resources"].append(sample)
                    stages.append(stage)
                    child.stdin.write("continue\n")
                    child.stdin.flush()
                child.wait(timeout=timeout)
                stderr = child.stderr.read()
                if child.returncode:
                    raise RuntimeError(f"probe exited {child.returncode}: {stderr}")
                if not stages or stages[-1]["stage"] != "released":
                    raise RuntimeError("probe did not complete every lifecycle stage")
            except BaseException:
                child.kill()
                child.wait()
                raise
    return {"clients": clients, "datatypes_per_client": datatypes,
            "backend": backend, "cleanup": cleanup, "stages": stages}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/release/examples/resource_probe"))
    parser.add_argument("--features", default="default=[]", help="Features used to build the binary (metadata only)")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--iterations", type=int, default=500)
    parser.add_argument("--settle", type=float, default=0.3)
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--retention-wait", type=float, default=11, help="Extra wait for the local drop-only retention control")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if platform.system() not in ("Darwin", "Linux"):
        parser.error("resource sampling supports macOS and Linux")
    if args.rounds < 1 or args.iterations < 1 or args.settle < 0 or args.timeout <= 0 or args.retention_wait < 0:
        parser.error("rounds, iterations and timeout must be positive; settle must be nonnegative")
    metadata = {"started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                "host": platform.platform(), "machine": platform.machine(),
                "rustc": command("rustc", "-Vv"), "commit": command("git", "rev-parse", "HEAD"),
                "dirty": bool(command("git", "status", "--porcelain")),
                "features": args.features, "binary": str(args.binary.resolve()),
                "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                "iterations": args.iterations, "settle_seconds": args.settle,
                "retention_wait_seconds": args.retention_wait,
                "sampling": "ps RSS KiB and OS thread count; three samples per checkpoint"}
    if platform.system() == "Darwin":
        metadata["hardware"] = command("sysctl", "hw.model", "hw.memsize", "hw.logicalcpu")
    results = {"metadata": metadata, "runs": []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    cases = [(1, 0), (1, 1), (2, 1), (4, 1), (8, 1), (16, 1), (1, 8), (1, 32), (1, 128), (4, 32)]
    scenarios = [(c, d, "local", "unsubscribe") for c, d in cases]
    scenarios += [(1, 8, "null", "drop"), (1, 8, "local", "drop")]
    for round_index in range(args.rounds):
        # Alternate order to expose drift without changing per-process isolation.
        for c, d, backend, cleanup in scenarios[::1 if round_index % 2 == 0 else -1]:
            run = probe(args.binary.resolve(), c, d, args.iterations, backend, cleanup, args.settle, args.timeout, args.retention_wait)
            run["round"] = round_index + 1
            results["runs"].append(run)
            args.output.write_text(json.dumps(results, indent=2) + "\n")
            print(f"round {round_index + 1}: {c} clients x {d} datatypes ({backend}, {cleanup})", flush=True)
    results["metadata"]["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    args.output.write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
