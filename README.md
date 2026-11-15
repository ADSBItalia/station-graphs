# ADSBItalia Station Graphs 📈📡

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Rust: 2021](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20Raspberry%20Pi%20%7C%20Docker-brightgreen.svg)]()
[![Community](https://img.shields.io/badge/Community-ADSBItalia-00f0ff.svg)](https://adsbitalia.it)

**ADSBItalia Station Graphs** is a modern, high-performance station telemetry and RF performance dashboard engineered in **Rust (Axum + Tokio)**. Built for ADS-B / MLAT stations running `readsb` or `dump1090-fa`, it replaces legacy, static server-rendered graphs with real-time, interactive avionics charts, RF signal analytics, feeder health monitoring, and dynamic polar antenna coverage.

![Dashboard Preview](https://raw.githubusercontent.com/ADSBItalia/station-graphs/main/web/favicon.svg)

---

## ✨ Features

- **⚡ Lightning-Fast Native Rust Backend**: Sub-millisecond JSON API responses with zero CPU and RAM overhead (~15 MB RAM footprint).
- **📡 Real-Time RF Signal Metrics**:
  - Peak & Median Signal Power (dBFS)
  - Noise Floor & Dynamic SNR (Signal-to-Noise Ratio in dB)
  - Message rates (msg/sec) & Position reception rates (pos/sec)
  - CPR error filtering & demodulation health
- **📈 Interactive Time-Series Graphs (Chart.js)**:
  - 1h, 6h, 24h, 48h, 7d, 30d, 90d, 1-Year historical exploration
  - Total aircraft & position tracking curves
  - Maximum reception range (NM / km) over time
  - SDR signal levels & dynamic noise floor
  - Host hardware metrics: CPU utilization % and core temperature
- **🌐 Autonomous Feeder Discovery**: Automatically detects active feeder services and Docker containers (`adsbitalia`, `ultrafeeder`, `flightradar24`, `flightaware`, `radarbox`, `planefinder`, `airplanes.live`, `adsb.lol`, `theairtraffic`, and more).
- **🧭 360° Antenna Reception Polar Diagram**: Real-time azimuthal coverage contour calculated dynamically from tracked aircraft positions.
- **🎨 Interactive Web Control Panel (`/settings`)**: Zero-JSON web configuration GUI with 1-click GPS auto-fill, decoder detection, and customizable color schemes (Cyan Avionics, Emerald Radar, Amber Tactical, Violet Horizon, Stealth Mono, Crimson Ops, or custom hex color pickers).
- **🌍 Bilingual & Units Toggle**: Seamless one-click switching between **English** and **Italian**, and between **Nautical (NM/ft)** and **Metric (km/m)** units.
- **🔗 Seamless Station Radar Integration**: Direct link to [ADSBItalia Station Radar](https://github.com/ADSBItalia/station-radar) running on port `8088`.

---

## 🚀 1-Click Automated Installation (Linux / Raspberry Pi)

Run this single command on your ADS-B receiver (Raspberry Pi, Debian, Ubuntu, Armbian):

```bash
bash <(curl -sL https://raw.githubusercontent.com/ADSBItalia/station-graphs/main/install.sh)
```

The installer will:
1. Detect your SDR decoder (`readsb` or `dump1090-fa`).
2. Build/install the optimized native Rust server.
3. Configure the systemd service (`adsbitalia-station-graphs.service`).
4. Launch the dashboard on port `8085` (`http://<YOUR_IP>:8085`).

---

## 🐳 Docker Deployment

You can run Station Graphs directly with Docker or Docker Compose. It seamlessly supports bare-metal ADS-B stacks (`readsb`, `dump1090-fa`) and containerized setups (**Ultrafeeder** / `docker-adsb-ultrafeeder`).

### Standard Bare-Metal / Native read:
```bash
git clone https://github.com/ADSBItalia/station-graphs.git
cd station-graphs
docker compose up -d --build
```

### With Ultrafeeder (Containerized Stack):
If running alongside Ultrafeeder in Docker, attach to its network and connect via HTTP:

```yaml
version: '3.8'

services:
  station-graphs:
    build: https://github.com/ADSBItalia/station-graphs.git#main
    image: ghcr.io/adsbitalia/station-graphs:latest
    container_name: adsbitalia-station-graphs
    restart: unless-stopped
    ports:
      - "8085:8085"
    volumes:
      # Mount Ultrafeeder's collectd persistent data directory:
      - /opt/adsb/ultrafeeder/graphs1090:/opt/adsb/ultrafeeder/graphs1090:ro
    environment:
      - PORT=8085
      - STATION_NAME=My ADS-B Station
      - TAR1090_URL=http://ultrafeeder/
      - COLLECTD_RRD_DIR=/opt/adsb/ultrafeeder/graphs1090
```

---

## 🛠️ Web Control Panel & Customization (Zero JSON Editing)

Station Graphs features a built-in **Avionics Web Settings Panel** accessible directly from your browser:

👉 **`http://<YOUR-IP>:8085/settings.html`**

No manual terminal or JSON editing is required:
* **🏷️ Station Identity**: Configure Station Name, Subtitle, and Antenna Coordinates (with 1-click GPS auto-detect).
* **🎨 Avionics Color Palette & Themes**: Choose between 6 avionics presets (*Cyan Avionics*, *Emerald Radar*, *Amber Tactical*, *Violet Horizon*, *Stealth Mono*, *Crimson Ops*) or customize your own primary, accent, and canvas colors with interactive color pickers and live real-time preview.
* **📡 Companion Radar Navigation**: Connect your local or remote Radar map (e.g. `http://<YOUR-IP>:8088/`) for seamless 1-click header navigation.
* **💾 Instant Safe Saving**: Settings are validated and applied live to all charts and polar graphs with zero service restart.

---

## 🔌 API Endpoints

Station Graphs provides clean JSON APIs for station monitoring and home automation (Home Assistant, Prometheus, custom dashboards):

| Endpoint | Method | Description |
|---|---|---|
| `/api/config` | `GET` | Station identity, coordinates, and version |
| `/api/live` | `GET` | Live RF metrics, aircraft count, CPR stats, system health, and active feeders |
| `/api/history?range=24h` | `GET` | Historical time-series arrays for charting (`1h`, `6h`, `24h`, `48h`, `7d`, `30d`, `90d`, `365d`) |
| `/api/coverage` | `GET` | 360° polar antenna coverage sectors (azimuth degrees & max reach) |
| `/health` | `GET` | Daemon health check |

---

## 🤝 Contributing & License

Open source under the [MIT License](LICENSE).
Contributions, bug reports, and pull requests are warmly welcome!

Developed by the **[ADSBItalia Team](https://adsbitalia.it)**.
