use axum::{
    extract::Query,
    http::{header, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{LazyLock, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct StationConfig {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub aircraft_json: String,
    pub stats_json: String,
    pub outline_json: String,
    pub dump_rrd: String,
    pub cpu_rrd: String,
    pub temp_rrd: String,
}

impl Default for StationConfig {
    fn default() -> Self {
        let mut name = std::env::var("STATION_NAME").unwrap_or_else(|_| "ADSB Station".to_string());
        let mut lat = std::env::var("STATION_LAT")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(44.769651);
        let mut lon = std::env::var("STATION_LON")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(11.930735);

        // Auto-detect from /run/readsb/receiver.json if available
        for p in &["/run/readsb/receiver.json", "/run/dump1090-fa/receiver.json"] {
            if let Ok(raw) = fs::read_to_string(p) {
                if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                    if let (Some(l_lat), Some(l_lon)) = (
                        val.get("lat").and_then(|v| v.as_f64()),
                        val.get("lon").and_then(|v| v.as_f64()),
                    ) {
                        lat = l_lat;
                        lon = l_lon;
                    }
                }
                break;
            }
        }

        // Auto-detect config file from /opt/adsbitalia-station-graphs/config.json or /etc/adsbitalia/feeder.conf
        if let Ok(raw) = fs::read_to_string("/opt/adsbitalia-station-graphs/config.json") {
            if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                if let Some(n) = val.get("station_name").and_then(|v| v.as_str()) {
                    name = n.to_string();
                }
                if let Some(l) = val.get("station_lat").and_then(|v| v.as_f64()) {
                    lat = l;
                }
                if let Some(l) = val.get("station_lon").and_then(|v| v.as_f64()) {
                    lon = l;
                }
            }
        }

        // Detect aircraft JSON directory
        let (aircraft_json, stats_json, outline_json) = if Path::new("/run/readsb/aircraft.json").exists() {
            (
                "/run/readsb/aircraft.json".to_string(),
                "/run/readsb/stats.json".to_string(),
                "/run/readsb/outline.json".to_string(),
            )
        } else if Path::new("/run/dump1090-fa/aircraft.json").exists() {
            (
                "/run/dump1090-fa/aircraft.json".to_string(),
                "/run/dump1090-fa/stats.json".to_string(),
                "/run/dump1090-fa/outline.json".to_string(),
            )
        } else {
            (
                "/run/readsb/aircraft.json".to_string(),
                "/run/readsb/stats.json".to_string(),
                "/run/readsb/outline.json".to_string(),
            )
        };

        // Detect collectd RRD directory
        let (dump_rrd, cpu_rrd, temp_rrd) = if Path::new("/run/collectd/localhost/dump1090-localhost").exists() {
            (
                "/run/collectd/localhost/dump1090-localhost".to_string(),
                "/run/collectd/localhost/aggregation-cpu-average".to_string(),
                "/run/collectd/localhost/table-localhost".to_string(),
            )
        } else if Path::new("/var/lib/collectd/rrd/localhost/dump1090-localhost").exists() {
            (
                "/var/lib/collectd/rrd/localhost/dump1090-localhost".to_string(),
                "/var/lib/collectd/rrd/localhost/aggregation-cpu-average".to_string(),
                "/var/lib/collectd/rrd/localhost/table-localhost".to_string(),
            )
        } else {
            (
                "/run/collectd/localhost/dump1090-localhost".to_string(),
                "/run/collectd/localhost/aggregation-cpu-average".to_string(),
                "/run/collectd/localhost/table-localhost".to_string(),
            )
        };

        Self {
            name,
            lat,
            lon,
            aircraft_json,
            stats_json,
            outline_json,
            dump_rrd,
            cpu_rrd,
            temp_rrd,
        }
    }
}

pub static CONFIG: LazyLock<RwLock<StationConfig>> =
    LazyLock::new(|| RwLock::new(StationConfig::default()));

static FEEDER_CACHE: LazyLock<RwLock<(Instant, Value)>> =
    LazyLock::new(|| RwLock::new((Instant::now() - std::time::Duration::from_secs(60), json!({}))));

static HISTORY_CACHE: LazyLock<RwLock<HashMap<String, (Instant, Value)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r_nm = 3440.065;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let delta_phi = (lat2 - lat1).to_radians();
    let delta_lambda = (lon2 - lon1).to_radians();

    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    r_nm * c
}

fn calculate_bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let delta_lambda = (lon2 - lon1).to_radians();

    let y = delta_lambda.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lambda.cos();
    let theta = y.atan2(x);
    (theta.to_degrees() + 360.0) % 360.0
}

fn get_system_metrics(temp_rrd: &str) -> Value {
    let mut cpu_temp: Option<f64> = None;
    if let Ok(raw) = fs::read_to_string("/sys/class/thermal/thermal_zone0/temp") {
        if let Ok(val) = raw.trim().parse::<f64>() {
            cpu_temp = Some((val / 100.0).round() / 10.0);
        }
    }
    if cpu_temp.is_none() {
        if let Ok(output) = Command::new("rrdtool")
            .args(["lastupdate", &format!("{}/gauge-cpu_temp.rrd", temp_rrd)])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(last_line) = text.lines().last() {
                    if let Some(part) = last_line.split(':').last() {
                        if let Ok(val) = part.trim().parse::<f64>() {
                            cpu_temp = Some((val / 100.0).round() / 10.0);
                        }
                    }
                }
            }
        }
    }

    let mut load_1m = 0.0;
    let mut load_5m = 0.0;
    if let Ok(raw) = fs::read_to_string("/proc/loadavg") {
        let parts: Vec<&str> = raw.split_whitespace().collect();
        if parts.len() >= 2 {
            load_1m = parts[0].parse::<f64>().unwrap_or(0.0);
            load_5m = parts[1].parse::<f64>().unwrap_or(0.0);
        }
    }

    let mut mem_percent = 0.0;
    let mut mem_used_mb = 0;
    let mut mem_total_mb = 0;
    if let Ok(raw) = fs::read_to_string("/proc/meminfo") {
        let mut total = 0;
        let mut avail = 0;
        for line in raw.lines() {
            if let Some((k, v)) = line.split_once(':') {
                let num = v.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
                match k.trim() {
                    "MemTotal" => total = num,
                    "MemAvailable" => avail = num,
                    _ => {}
                }
            }
        }
        if total > 0 {
            let used = total.saturating_sub(avail);
            mem_percent = ((used as f64 / total as f64) * 1000.0).round() / 10.0;
            mem_used_mb = used / 1024;
            mem_total_mb = total / 1024;
        }
    }

    let mut uptime_sec = 0;
    if let Ok(raw) = fs::read_to_string("/proc/uptime") {
        if let Some(first) = raw.split_whitespace().next() {
            uptime_sec = first.parse::<f64>().unwrap_or(0.0) as u64;
        }
    }

    json!({
        "cpu_temp": cpu_temp,
        "load_1m": load_1m,
        "load_5m": load_5m,
        "mem_percent": mem_percent,
        "mem_used_mb": mem_used_mb,
        "mem_total_mb": mem_total_mb,
        "uptime_sec": uptime_sec
    })
}

fn discover_feeders() -> Value {
    {
        let cache = FEEDER_CACHE.read().unwrap();
        if cache.0.elapsed().as_secs() < 8 && !cache.1.is_null() && cache.1.get("total").is_some() {
            return cache.1.clone();
        }
    }

    let known_containers: HashMap<&str, (&str, &str)> = HashMap::from([
        ("fr24feed", ("Flightradar24", "Beast / ADS-B")),
        ("piaware", ("FlightAware", "Beast / MLAT")),
        ("pfclient", ("PlaneFinder", "ADS-B")),
        ("rbfeeder", ("RadarBox", "Beast / MLAT")),
        ("opensky", ("OpenSky Network", "Raw Mode-S")),
        ("radarvirtuel", ("RadarVirtuel", "Mode-S + MLAT")),
        ("adsbhub", ("ADSBHub", "Raw Mode-S / SBS")),
        ("1090uk", ("1090uk", "Beast Ingest")),
        ("planewatch", ("PlaneWatch", "ADS-B Feed")),
        ("sdrmap", ("SDRmap.org", "Beast + MLAT")),
        ("acarshub", ("ACARSHub", "ACARS / VDL2 Hub")),
        ("acars_router", ("ACARS Router", "VDL2 / HFDL Router")),
        ("radarview", ("RadarView", "Native Rust Beast")),
        ("planeplotter", ("PlanePlotter", "UDP Uploader")),
        ("airframes", ("Airframes Satcom", "HFDL & Satcom Bridge")),
        ("live_mobile_mode_s", ("Live Mobile Mode-S", "Military Mode-S Feeder")),
        ("live-mobile-mode-s", ("Live Mobile Mode-S", "Military Mode-S Feeder")),
        ("dumpvdl2", ("dumpvdl2", "VDL2 Receiver & Decoder")),
        ("acars2pos", ("acars2pos", "ACARS Position Forwarder")),
    ]);

    let known_targets: HashMap<&str, &str> = HashMap::from([
        ("adsbitalia.it", "ADSBItalia Network"),
        ("mlat.adsbitalia.it", "ADSBItalia Network"),
        ("feed.airplanes.live", "Airplanes.live"),
        ("feed.adsb.lol", "adsb.lol"),
        ("feed.adsb.fi", "adsb.fi"),
        ("feed.adsb.win", "adsb.win"),
        ("mlat.adsb.win", "adsb.win"),
        ("feed.theairtraffic.com", "TheAirTraffic"),
        ("adsb.dataero.eu", "Dataero"),
        ("data.avdelphi.com", "AVDelphi"),
        ("dati.flyitalyadsb.com", "FlyItalyADSB"),
        ("feed.adsbiq.com", "ADSB IQ"),
        ("feed.avioadsb.org", "AvioADSB"),
        ("feed.flightportrait.com", "FlightPortrait"),
        ("feed.frlradar.nl", "FRL-RadaR"),
        ("feed.flyoverhead.com", "Fly Overhead"),
        ("feed.flyrealtraffic.com", "Fly Real Traffic"),
        ("feed.map.flights", "map.flights"),
        ("mlat.map.flights", "map.flights"),
        ("feed.mlat.uk", "MLAT.uk"),
        ("feed.planespotters.net", "Planespotters.net"),
        ("mlat.planespotters.net", "Planespotters.net"),
        ("feed.planetrack.ai", "PlaneTrack"),
        ("feed.skydex.online", "SkyDex"),
        ("feed.tryflightdeck.com", "FlightDeck"),
        ("feed.ulsterradar.com", "Ulster Radar"),
        ("mlat.ulsterradar.com", "Ulster Radar"),
        ("feed1.adsbexchange.com", "ADSBExchange"),
        ("feed.adsbexchange.com", "ADSBExchange"),
        ("skyfeed.hpradar.com", "HPRadar"),
    ]);

    let mut feeders: HashMap<String, Value> = HashMap::new();

    // 1. Docker containers
    if let Ok(output) = Command::new("docker")
        .args(["ps", "--format", "{{.Names}}|{{.Image}}|{{.Status}}"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.is_empty() {
                    continue;
                }
                let c_name = parts[0].trim();
                let c_image = if parts.len() > 1 { parts[1].trim() } else { "" };
                let c_status = if parts.len() > 2 { parts[2].trim() } else { "" };
                let is_up = c_status.contains("Up");

                if let Some(&(disp_name, f_type)) = known_containers.get(c_name) {
                    feeders.insert(
                        c_name.to_string(),
                        json!({
                            "id": c_name,
                            "name": disp_name,
                            "type": f_type,
                            "service": format!("docker:{}", c_name),
                            "status": if is_up { "active" } else { "offline" },
                            "active": is_up
                        }),
                    );
                } else if c_name.contains("feeder") || c_name.contains("feed") || c_image.contains("feeder") || c_image.contains("feed") {
                    let clean = c_name.replace("docker-", "").replace("-feed", "").replace("-feeder", "");
                    feeders.insert(
                        c_name.to_string(),
                        json!({
                            "id": c_name,
                            "name": clean,
                            "type": "Docker Feeder",
                            "service": format!("docker:{}", c_name),
                            "status": if is_up { "active" } else { "offline" },
                            "active": is_up
                        }),
                    );
                }
            }
        }
    }

    // 2. Ultrafeeder aggregators (adsb.im standard & custom)
    if let Ok(output) = Command::new("docker")
        .args(["exec", "ultrafeeder", "env"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut uf_config = "";
            for line in text.lines() {
                if let Some(stripped) = line.strip_prefix("ULTRAFEEDER_CONFIG=") {
                    uf_config = stripped;
                    break;
                }
            }

            let mut uf_targets: HashMap<String, (String, String, Vec<String>)> = HashMap::new();
            for entry in uf_config.split(';') {
                let parts: Vec<&str> = entry.split(',').map(|s| s.trim()).collect();
                if parts.len() < 3 {
                    continue;
                }
                let proto = parts[0];
                let host = parts[1];

                let disp_name = if let Some(&name) = known_targets.get(host) {
                    name.to_string()
                } else {
                    let base = host
                        .trim_start_matches("feed.")
                        .trim_start_matches("feed1.")
                        .trim_start_matches("mlat.")
                        .trim_start_matches("data.")
                        .trim_start_matches("dati.")
                        .trim_start_matches("skyfeed.")
                        .split('.')
                        .next()
                        .unwrap_or(host);
                    let mut c = base.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                };

                let key = disp_name.to_lowercase().replace([' ', '.'], "_");
                let entry_ref = uf_targets.entry(key.clone()).or_insert_with(|| {
                    (disp_name, host.to_string(), Vec::new())
                });
                let p_label = if proto == "mlat" { "MLAT" } else { "Beast" };
                if !entry_ref.2.contains(&p_label.to_string()) {
                    entry_ref.2.push(p_label.to_string());
                }
            }

            for (key, (disp_name, host, mut protocols)) in uf_targets {
                protocols.sort();
                let p_str = if protocols.is_empty() {
                    "Beast".to_string()
                } else {
                    protocols.join(" + ")
                };
                feeders.insert(
                    key.clone(),
                    json!({
                        "id": key,
                        "name": disp_name,
                        "type": format!("{} (adsb.im)", p_str),
                        "service": format!("ultrafeeder:{}", host),
                        "status": "active",
                        "active": true
                    }),
                );
            }
        }
    }

    // 3. Systemd host services
    if let Ok(output) = Command::new("systemctl")
        .args(["list-units", "--type=service", "--no-legend"])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                if let Some(svc) = line.split_whitespace().next() {
                    if svc.contains("airframes") || svc.contains("radarview") || svc.contains("ppup") || svc.contains("satcom") || svc.contains("planeplotter") || svc.contains("live-mobile-mode-s") || svc.contains("adsbitalia") {
                        let clean_id = svc.replace(".service", "").replace('-', "_");
                        if !feeders.contains_key(&clean_id) {
                            let mut is_active = false;
                            if let Ok(st_out) = Command::new("systemctl").args(["is-active", svc]).output() {
                                is_active = String::from_utf8_lossy(&st_out.stdout).trim() == "active";
                            }
                            let (disp_name, f_type) = if let Some(&(n, t)) = known_containers.get(clean_id.as_str()) {
                                (n.to_string(), t.to_string())
                            } else if let Some(&(n, t)) = known_containers.get(svc.replace(".service", "").as_str()) {
                                (n.to_string(), t.to_string())
                            } else if svc.contains("adsbitalia-feed") {
                                ("ADSBItalia Feeder".to_string(), "Native Beast Client".to_string())
                            } else if svc.contains("adsbitalia-mlat") {
                                ("ADSBItalia MLAT".to_string(), "Multilateration Client".to_string())
                            } else {
                                (svc.replace(".service", ""), "Native Host Service".to_string())
                            };
                            feeders.insert(
                                clean_id.clone(),
                                json!({
                                    "id": clean_id,
                                    "name": disp_name,
                                    "type": f_type,
                                    "service": svc,
                                    "status": if is_active { "active" } else { "offline" },
                                    "active": is_active
                                }),
                            );
                        }
                    }
                }
            }
        }
    }

    let mut list: Vec<Value> = feeders.into_values().collect();
    list.sort_by(|a, b| {
        let a_act = a.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
        let b_act = b.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
        if a_act != b_act {
            return b_act.cmp(&a_act);
        }
        let a_name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let b_name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        a_name.cmp(b_name)
    });

    let active_count = list.iter().filter(|f| f.get("active").and_then(|v| v.as_bool()).unwrap_or(false)).count();
    let total = list.len();

    let result = json!({
        "feeders": list,
        "total": total,
        "active_count": active_count,
        "all_healthy": active_count == total && total > 0
    });

    let mut cache = FEEDER_CACHE.write().unwrap();
    *cache = (Instant::now(), result.clone());
    result
}

pub async fn handle_config() -> impl IntoResponse {
    let cfg = CONFIG.read().unwrap().clone();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))],
        Json(json!({
            "station_name": cfg.name,
            "station_lat": cfg.lat,
            "station_lon": cfg.lon,
            "version": "1.0.0"
        })),
    )
}

pub async fn handle_live() -> impl IntoResponse {
    let cfg = CONFIG.read().unwrap().clone();
    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let system_metrics = get_system_metrics(&cfg.temp_rrd);
    let feeder_status = discover_feeders();

    let mut total_ac = 0;
    let mut with_pos = 0;
    let mut military_count = 0;
    let mut max_dist_nm = 0.0;
    let mut rssi_values: Vec<f64> = Vec::new();

    let mut alt_bins = HashMap::from([
        ("> 30.000 ft (Cruise)", 0),
        ("15.000 - 30.000 ft (Medium)", 0),
        ("3.000 - 15.000 ft (Low)", 0),
        ("< 3.000 ft (Approach)", 0),
    ]);

    let mut range_bins = HashMap::from([
        ("0 - 25 NM", 0),
        ("25 - 50 NM", 0),
        ("50 - 100 NM", 0),
        ("100 - 150 NM", 0),
        ("150 - 200 NM", 0),
        ("> 200 NM", 0),
    ]);

    if let Ok(raw) = fs::read_to_string(&cfg.aircraft_json) {
        if let Ok(ac_val) = serde_json::from_str::<Value>(&raw) {
            if let Some(list) = ac_val.get("aircraft").and_then(|v| v.as_array()) {
                total_ac = list.len();
                for a in list {
                    let db_flags = a.get("dbFlags").and_then(|v| v.as_u64()).unwrap_or(0);
                    if (db_flags & 1) != 0 {
                        military_count += 1;
                    }

                    if let Some(alt) = a.get("alt_baro").and_then(|v| v.as_f64()) {
                        if alt >= 30000.0 {
                            *alt_bins.entry("> 30.000 ft (Cruise)").or_insert(0) += 1;
                        } else if alt >= 15000.0 {
                            *alt_bins.entry("15.000 - 30.000 ft (Medium)").or_insert(0) += 1;
                        } else if alt >= 3000.0 {
                            *alt_bins.entry("3.000 - 15.000 ft (Low)").or_insert(0) += 1;
                        } else {
                            *alt_bins.entry("< 3.000 ft (Approach)").or_insert(0) += 1;
                        }
                    }

                    let lat_opt = a.get("lat").and_then(|v| v.as_f64());
                    let lon_opt = a.get("lon").and_then(|v| v.as_f64());
                    if let (Some(lat), Some(lon)) = (lat_opt, lon_opt) {
                        with_pos += 1;
                        let dist = haversine_nm(cfg.lat, cfg.lon, lat, lon);
                        if dist > max_dist_nm {
                            max_dist_nm = dist;
                        }
                        if dist <= 25.0 {
                            *range_bins.entry("0 - 25 NM").or_insert(0) += 1;
                        } else if dist <= 50.0 {
                            *range_bins.entry("25 - 50 NM").or_insert(0) += 1;
                        } else if dist <= 100.0 {
                            *range_bins.entry("50 - 100 NM").or_insert(0) += 1;
                        } else if dist <= 150.0 {
                            *range_bins.entry("100 - 150 NM").or_insert(0) += 1;
                        } else if dist <= 200.0 {
                            *range_bins.entry("150 - 200 NM").or_insert(0) += 1;
                        } else {
                            *range_bins.entry("> 200 NM").or_insert(0) += 1;
                        }
                    }

                    if let Some(rssi) = a.get("rssi").and_then(|v| v.as_f64()) {
                        rssi_values.push(rssi);
                    }
                }
            }
        }
    }

    let mut rssi_peak: Option<f64> = None;
    let mut rssi_median: Option<f64> = None;
    if !rssi_values.is_empty() {
        rssi_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        rssi_peak = rssi_values.last().copied().map(|v| (v * 10.0).round() / 10.0);
        let mid = rssi_values.len() / 2;
        rssi_median = Some((rssi_values[mid] * 10.0).round() / 10.0);
    }

    let mut msgs_sec = 0.0;
    let mut pos_sec = 0.0;
    let mut total_messages = 0u64;
    let mut bits_112 = 0u64;
    let mut bits_56 = 0u64;
    let mut gain_db = 40.2;
    let mut est_ppm = -4.0;
    let mut snr_db = 20.0;
    let mut rf_signal = -10.0;
    let mut rf_noise = -30.0;

    let mut cpr_global_ok = 0u64;
    let mut cpr_global_bad = 0u64;
    let mut cpr_local_ok = 0u64;
    let mut cpr_filtered = 0u64;

    if let Ok(raw) = fs::read_to_string(&cfg.stats_json) {
        if let Ok(st) = serde_json::from_str::<Value>(&raw) {
            let last1min = st.get("last1min");
            let total = st.get("total");

            if let Some(sec) = last1min {
                if let Some(m) = sec.get("messages").and_then(|v| v.as_f64()) {
                    msgs_sec = (m / 60.0 * 10.0).round() / 10.0;
                }
                if let Some(p) = sec.get("positions_with_position").and_then(|v| v.as_f64()) {
                    pos_sec = (p / 60.0 * 10.0).round() / 10.0;
                }
                if let Some(s) = sec.get("peak_signal").and_then(|v| v.as_f64()) {
                    rf_signal = (s * 10.0).round() / 10.0;
                }
                if let Some(n) = sec.get("noise").and_then(|v| v.as_f64()) {
                    rf_noise = (n * 10.0).round() / 10.0;
                }
                snr_db = ((rf_signal - rf_noise) * 10.0).round() / 10.0;
            }

            if let Some(tot) = total {
                if let Some(m) = tot.get("messages").and_then(|v| v.as_u64()) {
                    total_messages = m;
                }
                if let Some(b112) = tot.get("messages_112bit").and_then(|v| v.as_u64()) {
                    bits_112 = b112;
                }
                if let Some(b56) = tot.get("messages_56bit").and_then(|v| v.as_u64()) {
                    bits_56 = b56;
                }
                if let Some(cpr) = tot.get("cpr") {
                    cpr_global_ok = cpr.get("global_ok").and_then(|v| v.as_u64()).unwrap_or(0);
                    cpr_global_bad = cpr.get("global_bad").and_then(|v| v.as_u64()).unwrap_or(0);
                    cpr_local_ok = cpr.get("local_ok").and_then(|v| v.as_u64()).unwrap_or(0);
                    cpr_filtered = cpr.get("filtered").and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }
        }
    }

    let payload = json!({
        "timestamp": now_ts,
        "station": {
            "name": cfg.name,
            "lat": cfg.lat,
            "lon": cfg.lon
        },
        "live": {
            "aircraft_total": total_ac,
            "aircraft_with_pos": with_pos,
            "aircraft_military": military_count,
            "max_distance_nm": (max_dist_nm * 10.0).round() / 10.0,
            "messages_sec": msgs_sec,
            "positions_sec": pos_sec,
            "total_messages": total_messages,
            "bits_112": bits_112,
            "bits_56": bits_56,
            "gain_db": gain_db,
            "estimated_ppm": est_ppm,
            "snr_db": snr_db,
            "rf_signal_dbfs": rf_signal,
            "rf_noise_dbfs": rf_noise,
            "rssi_peak": rssi_peak,
            "rssi_median": rssi_median,
            "cpr_stats": {
                "global_ok": cpr_global_ok,
                "global_bad": cpr_global_bad,
                "local_ok": cpr_local_ok,
                "filtered": cpr_filtered
            }
        },
        "distributions": {
            "altitude": alt_bins,
            "range": range_bins
        },
        "feeders": feeder_status,
        "system": system_metrics
    });

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))],
        Json(payload),
    )
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub range: Option<String>,
}

pub async fn handle_history(Query(q): Query<HistoryQuery>) -> impl IntoResponse {
    let cfg = CONFIG.read().unwrap().clone();
    let range = q.range.unwrap_or_else(|| "24h".to_string());
    let range_clean = match range.as_str() {
        "1h" | "6h" | "24h" | "48h" | "7d" | "14d" | "30d" | "90d" | "365d" => range.as_str(),
        _ => "24h",
    };

    let ttl_secs = match range_clean {
        "1h" => 15,
        "6h" => 30,
        "24h" => 60,
        "48h" => 120,
        "7d" => 300,
        _ => 600,
    };

    {
        let cache = HISTORY_CACHE.read().unwrap();
        if let Some((instant, val)) = cache.get(range_clean) {
            if instant.elapsed().as_secs() < ttl_secs {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))],
                    Json(val.clone()),
                );
            }
        }
    }

    let (start_arg, step_arg) = match range_clean {
        "1h" => ("-1h", "60"),
        "6h" => ("-6h", "120"),
        "24h" => ("-24h", "300"),
        "48h" => ("-48h", "600"),
        "7d" => ("-7d", "1800"),
        "14d" => ("-14d", "3600"),
        "30d" => ("-30d", "7200"),
        "90d" => ("-90d", "21600"),
        "365d" => ("-365d", "86400"),
        _ => ("-24h", "300"),
    };

    let cmd_output = Command::new("rrdtool")
        .args([
            "xport",
            "--json",
            "--start",
            start_arg,
            "--step",
            step_arg,
            &format!("DEF:ac_tot={}/dump1090_aircraft-recent.rrd:total:AVERAGE", cfg.dump_rrd),
            &format!("DEF:ac_pos={}/dump1090_aircraft-recent.rrd:positions:AVERAGE", cfg.dump_rrd),
            &format!("DEF:msg_sec={}/dump1090_messages-local_accepted.rrd:value:AVERAGE", cfg.dump_rrd),
            &format!("DEF:max_rng={}/dump1090_range-max_range.rrd:value:AVERAGE", cfg.dump_rrd),
            "CDEF:max_rng_nm=max_rng,1852,/",
            &format!("DEF:sig_peak={}/dump1090_dbfs-peak_signal.rrd:value:AVERAGE", cfg.dump_rrd),
            &format!("DEF:sig_med={}/dump1090_dbfs-median.rrd:value:AVERAGE", cfg.dump_rrd),
            &format!("DEF:cpu_idle={}/cpu-idle.rrd:value:AVERAGE", cfg.cpu_rrd),
            "CDEF:cpu_pct=100,cpu_idle,-",
            &format!("DEF:raw_temp={}/gauge-cpu_temp.rrd:value:AVERAGE", cfg.temp_rrd),
            "CDEF:cpu_temp=raw_temp,1000,/",
            "XPORT:ac_tot:total_aircraft",
            "XPORT:ac_pos:pos_aircraft",
            "XPORT:msg_sec:messages_sec",
            "XPORT:max_rng_nm:max_range_nm",
            "XPORT:sig_peak:peak_dbfs",
            "XPORT:sig_med:median_dbfs",
            "XPORT:cpu_pct:cpu_pct",
            "XPORT:cpu_temp:cpu_temp",
        ])
        .output();

    match cmd_output {
        Ok(out) if out.status.success() => {
            if let Ok(rrd_val) = serde_json::from_slice::<Value>(&out.stdout) {
                let meta = rrd_val.get("meta").cloned().unwrap_or(json!({}));
                let start_ts = meta.get("start").and_then(|v| v.as_i64()).unwrap_or(0);
                let step = meta.get("step").and_then(|v| v.as_i64()).unwrap_or(60);
                let rows = rrd_val.get("data").and_then(|v| v.as_array()).cloned().unwrap_or_default();

                let mut timestamps: Vec<i64> = Vec::with_capacity(rows.len());
                let mut s_tot: Vec<Option<f64>> = Vec::with_capacity(rows.len());
                let mut s_pos: Vec<Option<f64>> = Vec::with_capacity(rows.len());
                let mut s_msg: Vec<Option<f64>> = Vec::with_capacity(rows.len());
                let mut s_rng: Vec<Option<f64>> = Vec::with_capacity(rows.len());
                let mut s_sig_pk: Vec<Option<f64>> = Vec::with_capacity(rows.len());
                let mut s_sig_med: Vec<Option<f64>> = Vec::with_capacity(rows.len());
                let mut s_cpu: Vec<Option<f64>> = Vec::with_capacity(rows.len());
                let mut s_temp: Vec<Option<f64>> = Vec::with_capacity(rows.len());

                let mut curr_ts = start_ts;
                for row in rows {
                    curr_ts += step;
                    timestamps.push(curr_ts);
                    if let Some(arr) = row.as_array() {
                        let parse_item = |idx: usize| -> Option<f64> {
                            arr.get(idx).and_then(|v| v.as_f64()).map(|n| (n * 100.0).round() / 100.0)
                        };
                        s_tot.push(parse_item(0));
                        s_pos.push(parse_item(1));
                        s_msg.push(parse_item(2));
                        s_rng.push(parse_item(3));
                        s_sig_pk.push(parse_item(4));
                        s_sig_med.push(parse_item(5));
                        s_cpu.push(parse_item(6));
                        s_temp.push(parse_item(7));
                    }
                }

                let result = json!({
                    "range": range_clean,
                    "step": step,
                    "timestamps": timestamps,
                    "series": {
                        "total_aircraft": s_tot,
                        "pos_aircraft": s_pos,
                        "messages_sec": s_msg,
                        "max_range_nm": s_rng,
                        "peak_dbfs": s_sig_pk,
                        "median_dbfs": s_sig_med,
                        "cpu_pct": s_cpu,
                        "cpu_temp": s_temp
                    },
                    "cached_at": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
                });

                let mut cache = HISTORY_CACHE.write().unwrap();
                cache.insert(range_clean.to_string(), (Instant::now(), result.clone()));

                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))],
                    Json(result),
                );
            }
        }
        _ => {}
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))],
        Json(json!({"error": "Failed to query RRD history", "range": range_clean})),
    )
}

pub async fn handle_coverage() -> impl IntoResponse {
    let cfg = CONFIG.read().unwrap().clone();
    if let Ok(raw) = fs::read_to_string(&cfg.outline_json) {
        if let Ok(data) = serde_json::from_str::<Value>(&raw) {
            let mut points: Vec<Value> = Vec::new();
            if let Some(pts) = data.get("actualRange").and_then(|a| a.get("last24h")).and_then(|l| l.get("points")).and_then(|p| p.as_array()) {
                points = pts.clone();
            } else if let Some(pts) = data.get("points").and_then(|p| p.as_array()) {
                points = pts.clone();
            }

            let mut sectors: HashMap<i32, f64> = HashMap::new();
            for i in 0..36 {
                sectors.insert(i * 10, 0.0);
            }

            for p in &points {
                if let Some(arr) = p.as_array() {
                    if arr.len() >= 2 {
                        if let (Some(lat), Some(lon)) = (arr[0].as_f64(), arr[1].as_f64()) {
                            let dist_nm = haversine_nm(cfg.lat, cfg.lon, lat, lon);
                            let brg = calculate_bearing(cfg.lat, cfg.lon, lat, lon);
                            let sec_idx = ((brg / 10.0).floor() as i32 * 10) % 360;
                            if let Some(cur) = sectors.get_mut(&sec_idx) {
                                if dist_nm > *cur {
                                    *cur = (dist_nm * 10.0).round() / 10.0;
                                }
                            }
                        }
                    }
                }
            }

            let mut keys: Vec<i32> = sectors.keys().copied().collect();
            keys.sort();

            let sector_labels: Vec<String> = keys.iter().map(|k| format!("{}°", k)).collect();
            let sector_values: Vec<f64> = keys.iter().map(|k| sectors[k]).collect();
            let max_val = sector_values.iter().copied().fold(0.0f64, f64::max);

            let res = json!({
                "station": {"lat": cfg.lat, "lon": cfg.lon, "name": cfg.name},
                "sectors_deg": keys,
                "sector_labels": sector_labels,
                "sector_ranges_nm": sector_values,
                "max_range_nm": max_val,
                "points_count": points.len()
            });

            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))],
                Json(res),
            );
        }
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"))],
        Json(json!({"error": "Failed to read outline.json"})),
    )
}
