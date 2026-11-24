#!/usr/bin/env python3
"""Find Axis devices on the current Wi-Fi/LAN using Python's standard library."""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import http.client
import ipaddress
import json
import math
import platform
import re
import ssl
import subprocess
import sys


def detect_network(interface=None):
    """Read the Wi-Fi subnet on macOS, or the default interface subnet on Linux."""
    if platform.system() == "Darwin":
        if not interface:
            ports = subprocess.check_output(
                ["/usr/sbin/networksetup", "-listallhardwareports"], text=True
            )
            match = re.search(r"Hardware Port: (?:Wi-Fi|AirPort)\nDevice: (\S+)", ports)
            if not match:
                raise ValueError("No Wi-Fi interface found; pass --interface or --network.")
            interface = match[1]
        config = subprocess.check_output(["/sbin/ifconfig", interface], text=True)
        match = re.search(r"\binet (\S+) netmask (\S+)", config)
        if not match:
            raise ValueError(f"No IPv4 address on {interface}; connect to Wi-Fi first.")
        address, mask = match.groups()
        if mask.startswith("0x"):
            mask = str(ipaddress.IPv4Address(int(mask, 16)))
        return ipaddress.IPv4Network(f"{address}/{mask}", strict=False)

    if platform.system() == "Linux":
        if not interface:
            routes = json.loads(subprocess.check_output(
                ["ip", "-j", "-4", "route", "show", "default"], text=True
            ))
            if not routes:
                raise ValueError("No default route; pass --interface or --network.")
            interface = routes[0]["dev"]
        links = json.loads(subprocess.check_output(
            ["ip", "-j", "-4", "addr", "show", "dev", interface], text=True
        ))
        for link in links:
            for address in link.get("addr_info", []):
                if address["family"] == "inet":
                    return ipaddress.IPv4Network(
                        f"{address['local']}/{address['prefixlen']}", strict=False
                    )
        raise ValueError(f"No IPv4 address on {interface}.")

    raise ValueError("Automatic detection needs macOS or Linux; pass --network.")


def probe(address, timeout, tls_context):
    """Identify a device using public VAPIX info or an Axis authentication realm."""
    requests = (
        ("POST", "/axis-cgi/basicdeviceinfo.cgi", json.dumps({
            "apiVersion": "1.2", "method": "getAllUnrestrictedProperties"
        })),
        ("GET", "/axis-cgi/param.cgi?action=list&group=Brand", None),
    )
    for scheme in ("http", "https"):
        if scheme == "https":
            connection = http.client.HTTPSConnection(
                str(address), timeout=timeout, context=tls_context
            )
        else:
            connection = http.client.HTTPConnection(str(address), timeout=timeout)
        try:
            for method, path, body in requests:
                connection.request(method, path, body=body, headers={
                    "Content-Type": "application/json", "Connection": "close"
                })
                response = connection.getresponse()
                auth = response.getheader("WWW-Authenticate", "")
                if re.search(r'\brealm="AXIS[_ -]', auth, re.IGNORECASE):
                    return address, f"{scheme}://{address}", "AXIS (login required)"
                payload = response.read(32768).decode("utf-8", errors="replace")
                connection.close()
                try:
                    properties = json.loads(payload).get("data", {}).get("propertyList", {})
                    if properties.get("Brand", "").upper() == "AXIS":
                        name = properties.get("ProdFullName") or "AXIS"
                        return address, f"{scheme}://{address}", name
                except (ValueError, AttributeError, TypeError):
                    pass
                if re.search(r"^root\.Brand\.Brand=AXIS\s*$", payload, re.MULTILINE):
                    name = re.search(r"^root\.Brand\.ProdFullName=(.+)$", payload, re.MULTILINE)
                    return address, f"{scheme}://{address}", name[1].strip() if name else "AXIS"
        except (OSError, http.client.HTTPException):
            pass
        finally:
            connection.close()
    return None


def main():
    """Scan one IPv4 subnet and print web links and standard Axis RTSP URLs."""
    parser = argparse.ArgumentParser(description=__doc__)
    target = parser.add_mutually_exclusive_group()
    target.add_argument("--network", help="IPv4 subnet, e.g. 192.168.1.0/24")
    target.add_argument("--interface", help="Network interface instead of automatic selection")
    parser.add_argument("--timeout", type=float, default=1.0, help="Socket timeout in seconds (default: 1)")
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be a positive, finite number")
    try:
        network = (
            ipaddress.IPv4Network(args.network, strict=False)
            if args.network else detect_network(args.interface)
        )
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    if network.num_addresses > 4096:
        parser.error(f"{network} is too large; pass a smaller --network (at most /20)")

    # Cameras often use self-signed certificates. No credentials are sent.
    tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    tls_context.check_hostname = False
    tls_context.verify_mode = ssl.CERT_NONE
    print(f"Scanning {network} for Axis devices (HTTP/HTTPS)...", flush=True)
    found = []
    pool = ThreadPoolExecutor(max_workers=64)
    try:
        futures = [pool.submit(probe, address, args.timeout, tls_context) for address in network.hosts()]
        for future in as_completed(futures):
            result = future.result()
            if result:
                found.append(result)
                address, url, name = result
                print(f"\n{name}\n  Web:  {url}\n  RTSP: rtsp://{address}/axis-media/media.amp", flush=True)
    finally:
        # On Ctrl-C, cancel queued hosts instead of waiting for the entire subnet.
        pool.shutdown(wait=True, cancel_futures=True)
    if not found:
        print("No Axis devices found. Check the subnet and Wi-Fi client isolation, or try --timeout 3.")
    else:
        print(f"\nFound {len(found)} Axis device(s). RTSP URLs use the default path/port and may need credentials.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
