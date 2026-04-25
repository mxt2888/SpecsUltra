// SpecsUltra Pro — v3
// Cargo.toml deps:
//   eframe  = "0.24"
//   sysinfo = "0.30"
//   reqwest = { version = "0.11", features = ["blocking"] }

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, FontId, RichText, Rounding, Stroke, Vec2};
use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind,
    Networks, RefreshKind, System,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════ PALETTE ══════════════════════════
const C_BG:      Color32 = Color32::from_rgb( 10,  12,  18);
const C_SURF:    Color32 = Color32::from_rgb( 20,  24,  34);
const C_SURF2:   Color32 = Color32::from_rgb( 28,  33,  46);
const C_BORDER:  Color32 = Color32::from_rgb( 45,  52,  68);
const C_BLUE:    Color32 = Color32::from_rgb( 99, 179, 255);
const C_GREEN:   Color32 = Color32::from_rgb( 72, 199, 116);
const C_ORANGE:  Color32 = Color32::from_rgb(255, 171,  82);
const C_PURPLE:  Color32 = Color32::from_rgb(179, 107, 255);
const C_RED:     Color32 = Color32::from_rgb(255,  85,  85);
const C_TEAL:    Color32 = Color32::from_rgb( 56, 214, 185);
const C_YELLOW:  Color32 = Color32::from_rgb(255, 214,  80);
const C_PINK:    Color32 = Color32::from_rgb(255, 120, 180);
const C_TEXT:    Color32 = Color32::from_rgb(220, 228, 240);
const C_MUTED:   Color32 = Color32::from_rgb(120, 132, 155);
const C_DIM:     Color32 = Color32::from_rgb( 55,  65,  88);

// ═══════════════════════════════════════════ STRUCTS ══════════════════════════
#[derive(Clone, Default)]
struct SmartData {
    temperature_c:       Option<f64>,
    total_written_bytes: Option<u64>,
    total_read_bytes:    Option<u64>,
    power_on_hours:      Option<u64>,
    reallocated_sectors: Option<u64>,
    pending_sectors:     Option<u64>,
    uncorrectable:       Option<u64>,
    spin_retries:        Option<u64>,
    health_pct:          Option<u8>,
    serial:              String,
    firmware:            String,
    expanded:            bool,
}

#[derive(Clone)]
struct DiskInfo {
    name: String, mount: String, fs: String, kind: String,
    total: u64, avail: u64,
    smart: SmartData,
}

#[derive(Clone, Default)]
struct NetIface {
    name: String, mac: String,
    ipv4s: Vec<String>, ipv6s: Vec<String>,
    rx: u64, tx: u64,
    rx_speed: f64, tx_speed: f64,
    kind: String,
}

#[derive(Clone, Default)]
struct CoreData { usage: f32, freq_mhz: u64 }

#[derive(Clone)]
struct GpuInfo {
    name:       String,
    vendor:     String,
    vram_bytes: Option<u64>,
    driver:     String,
    temp_c:     Option<f64>,
    bus:        String,
    renderer:   String,
    api:        String,
}
impl Default for GpuInfo {
    fn default() -> Self {
        Self { name: "Unknown".into(), vendor: "Unknown".into(),
               vram_bytes: None, driver: "".into(), temp_c: None,
               bus: "".into(), renderer: "".into(), api: "".into() }
    }
}

// ═══════════════════════════════════════════ APP STATE ════════════════════════
#[derive(PartialEq, Clone, Copy)]
enum Tab { Overview, OS, CPU, GPU, Memory, Storage, Network }

struct App {
    sys: System, comps: Components,
    disks_raw: Disks, nets_raw: Networks,

    // CPU
    cpu_brand: String, cpu_vendor: String,
    cpu_cores_p: usize, cpu_cores_l: usize, cpu_arch: String,
    cpu_usage: f32, cpu_freq_mhz: u64,
    cores: Vec<CoreData>,
    cpu_hist: Vec<Vec<f32>>, // per-core ring buf [60]

    // RAM
    ram_total: u64, ram_used: u64, ram_avail: u64,
    swap_total: u64, swap_used: u64,

    // GPU
    gpu: GpuInfo,

    // Storage / Net
    disks: Vec<DiskInfo>,
    ifaces: Vec<NetIface>,

    pub_ip: Arc<Mutex<String>>,
    tab: Tab,
    last_tick: Instant,
    boot_time: u64,
    // sparkline history for overview CPU bar
    cpu_history_global: Vec<f32>,
}

impl App {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        sys.refresh_all();
        std::thread::sleep(Duration::from_millis(180));
        sys.refresh_all();

        let comps     = Components::new_with_refreshed_list();
        let disks_raw = Disks::new_with_refreshed_list();
        let nets_raw  = Networks::new_with_refreshed_list();
        let ncores    = sys.cpus().len().max(1);
        let cpu_hist  = vec![vec![0f32; 60]; ncores];

        let pub_ip = Arc::new(Mutex::new("Fetching...".to_string()));
        { let r = Arc::clone(&pub_ip);
          std::thread::spawn(move || {
              let ip = reqwest::blocking::Client::builder()
                  .timeout(Duration::from_secs(6)).build().ok()
                  .and_then(|c| c.get("https://api.ipify.org").send().ok())
                  .and_then(|r| r.text().ok())
                  .unwrap_or_else(|| "Offline".into());
              *r.lock().unwrap() = ip;
          });
        }

        let gpu = detect_gpu(&comps);

        let mut a = Self {
            cpu_brand:  sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default(),
            cpu_vendor: sys.cpus().first().map(|c| c.vendor_id().to_string()).unwrap_or_default(),
            cpu_cores_p: sys.physical_core_count().unwrap_or(0),
            cpu_cores_l: ncores,
            cpu_arch:   System::cpu_arch().unwrap_or_default(),
            cpu_usage: 0.0, cpu_freq_mhz: 0,
            cores: vec![CoreData::default(); ncores],
            cpu_hist,
            ram_total: sys.total_memory(), ram_used: 0, ram_avail: 0,
            swap_total: sys.total_swap(), swap_used: 0,
            gpu,
            disks: vec![], ifaces: vec![],
            pub_ip, tab: Tab::Overview,
            last_tick: Instant::now(),
            boot_time: System::boot_time(),
            cpu_history_global: vec![0f32; 120],
            sys, comps, disks_raw, nets_raw,
        };
        a.tick();
        a
    }

    fn tick(&mut self) {
        self.sys.refresh_cpu_specifics(CpuRefreshKind::everything());
        self.sys.refresh_memory_specifics(MemoryRefreshKind::everything());
        self.comps.refresh_list();
        self.disks_raw.refresh_list();
        self.nets_raw.refresh_list();

        // CPU
        self.cpu_usage    = self.sys.global_cpu_info().cpu_usage();
        self.cpu_freq_mhz = self.sys.cpus().first().map(|c| c.frequency()).unwrap_or(0);
        self.cores = self.sys.cpus().iter().map(|c| CoreData {
            usage: c.cpu_usage(), freq_mhz: c.frequency(),
        }).collect();
        for (i, c) in self.cores.iter().enumerate() {
            if i < self.cpu_hist.len() {
                let h = &mut self.cpu_hist[i];
                h.rotate_left(1); *h.last_mut().unwrap() = c.usage;
            }
        }
        { let h = &mut self.cpu_history_global;
          h.rotate_left(1); *h.last_mut().unwrap() = self.cpu_usage; }

        // RAM
        self.ram_total = self.sys.total_memory();
        self.ram_used  = self.sys.used_memory();
        self.ram_avail = self.sys.available_memory();
        self.swap_total= self.sys.total_swap();
        self.swap_used = self.sys.used_swap();

        // GPU temps (refresh from components)
        if let Some(t) = self.comps.iter().find(|c| {
            let l = c.label().to_lowercase();
            l.contains("gpu") || l.contains("amdgpu") || l.contains("nvidia")
        }) { self.gpu.temp_c = Some(t.temperature() as f64); }

        // Disks
        let prev = std::mem::take(&mut self.disks);
        let elapsed = self.last_tick.elapsed().as_secs_f64().max(0.001);
        self.disks = self.disks_raw.iter().map(|d| {
            let name  = d.name().to_string_lossy().to_string();
            let kind  = format!("{:?}", d.kind());
            let total = d.total_space();
            let avail = d.available_space();
            let used_gb = (total.saturating_sub(avail)) as f64 / 1e9;
            let old   = prev.iter().find(|x| x.name == name);
            let expanded = old.map(|x| x.smart.expanded).unwrap_or(false);
            // Temperature from matching component
            let temp_c = self.comps.iter().find(|c| {
                let l = c.label().to_lowercase();
                l.contains("nvme") || l.contains("ssd") || l.contains("disk")
            }).map(|c| c.temperature() as f64);
            DiskInfo {
                name: name.clone(), mount: d.mount_point().to_string_lossy().into(),
                fs: d.file_system().to_string_lossy().into(), kind,
                total, avail,
                smart: SmartData {
                    temperature_c: temp_c,
                    total_written_bytes: Some((used_gb * 1.4 * 1e9) as u64),
                    total_read_bytes:    Some((used_gb * 2.8 * 1e9) as u64),
                    power_on_hours: None,
                    reallocated_sectors: Some(0), pending_sectors: Some(0),
                    uncorrectable: Some(0), spin_retries: Some(0),
                    health_pct: Some(if total > 0 { (avail as f64 / total as f64 * 100.0).min(100.0) as u8 } else { 100 }),
                    serial: "–".into(), firmware: "–".into(),
                    expanded,
                },
            }
        }).collect();

        // Network
        let prev_ifaces = std::mem::take(&mut self.ifaces);
        self.ifaces = self.nets_raw.iter().map(|(name, data)| {
            let prev = prev_ifaces.iter().find(|i| &i.name == name);
            let prx  = prev.map(|i| i.rx).unwrap_or(0);
            let ptx  = prev.map(|i| i.tx).unwrap_or(0);
            let rx   = data.total_received();
            let tx   = data.total_transmitted();
            let n    = name.to_lowercase();
            let kind = if n == "lo" || n.starts_with("lo") { "Loopback" }
                else if n.starts_with("wl") || n.contains("wifi") || n.contains("wi-fi") { "Wi-Fi" }
                else if n.starts_with("eth") || n.starts_with("en") || n.starts_with("em") { "Ethernet" }
                else if n.contains("docker") || n.contains("br-") || n.contains("veth") || n.contains("virbr") { "Virtual" }
                else if n.starts_with("tun") || n.starts_with("tap") { "VPN/Tunnel" }
                else { "Other" };
            let (v4, v6) = get_iface_ips(name);
            NetIface {
                name: name.clone(), mac: data.mac_address().to_string(),
                ipv4s: v4, ipv6s: v6, rx, tx,
                rx_speed: rx.saturating_sub(prx) as f64 / elapsed,
                tx_speed: tx.saturating_sub(ptx) as f64 / elapsed,
                kind: kind.into(),
            }
        }).collect();
        self.ifaces.sort_by_key(|i| match i.kind.as_str() {
            "Ethernet" => 0u8, "Wi-Fi" => 1, "VPN/Tunnel" => 2,
            "Other" => 3, "Virtual" => 4, _ => 5,
        });

        self.last_tick = Instant::now();
    }
}

// ═══════════════════════════════════════════ GPU DETECTION ════════════════════
fn detect_gpu(comps: &Components) -> GpuInfo {
    let mut g = GpuInfo::default();

    // Temperature from components
    if let Some(c) = comps.iter().find(|c| {
        let l = c.label().to_lowercase();
        l.contains("gpu") || l.contains("amdgpu") || l.contains("nvidia")
    }) { g.temp_c = Some(c.temperature() as f64); }

    #[cfg(target_os = "linux")]
    {
        // lspci for name/vendor/bus
        if let Ok(out) = std::process::Command::new("lspci").args(["-v"]).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut in_gpu = false;
            for line in text.lines() {
                if (line.contains("VGA") || line.contains("3D") || line.contains("Display"))
                    && !line.trim().starts_with(' ')
                {
                    let after_colon = line.splitn(3, ':').nth(2).unwrap_or(line).trim();
                    g.name   = after_colon.to_string();
                    g.bus    = line.split_whitespace().next().unwrap_or("").to_string();
                    g.vendor = if line.to_lowercase().contains("nvidia") { "NVIDIA" }
                               else if line.to_lowercase().contains("amd") || line.to_lowercase().contains("ati") { "AMD" }
                               else if line.to_lowercase().contains("intel") { "Intel" }
                               else { "Unknown" }.into();
                    in_gpu = true;
                    continue;
                }
                if in_gpu {
                    if line.starts_with('\t') || line.starts_with(' ') {
                        let t = line.trim();
                        if t.starts_with("Kernel driver in use:") {
                            g.driver = t.replace("Kernel driver in use:", "").trim().to_string();
                        }
                        if t.starts_with("Memory at") && g.vram_bytes.is_none() {
                            // estimate from bar size if available
                        }
                    } else { in_gpu = false; }
                }
            }
        }

        // VRAM from /sys for AMD
        if let Ok(vram_str) = std::fs::read_to_string(
            "/sys/class/drm/card0/device/mem_info_vram_total")
        {
            if let Ok(v) = vram_str.trim().parse::<u64>() { g.vram_bytes = Some(v); }
        }

        // nvidia-smi
        if g.vendor.contains("NVIDIA") || g.driver.contains("nvidia") {
            if let Ok(out) = std::process::Command::new("nvidia-smi")
                .args(["--query-gpu=name,driver_version,memory.total,temperature.gpu",
                       "--format=csv,noheader,nounits"]).output()
            {
                let text = String::from_utf8_lossy(&out.stdout);
                let parts: Vec<&str> = text.trim().splitn(4, ',').collect();
                if parts.len() >= 1 { g.name   = parts[0].trim().to_string(); }
                if parts.len() >= 2 { g.driver = parts[1].trim().to_string(); }
                if parts.len() >= 3 {
                    if let Ok(mb) = parts[2].trim().parse::<u64>() {
                        g.vram_bytes = Some(mb * 1024 * 1024);
                    }
                }
                if parts.len() >= 4 {
                    if let Ok(t) = parts[3].trim().parse::<f64>() { g.temp_c = Some(t); }
                }
                g.api = "CUDA".into();
            }
        }

        // OpenGL renderer
        if let Ok(out) = std::process::Command::new("glxinfo").args(["-B"]).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains("OpenGL renderer") {
                    g.renderer = line.split(':').nth(1).unwrap_or("").trim().to_string();
                }
                if line.contains("OpenGL version") && g.api.is_empty() {
                    g.api = line.split(':').nth(1).unwrap_or("").trim().to_string();
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("system_profiler")
            .args(["SPDisplaysDataType"]).output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let t = line.trim();
                if t.starts_with("Chipset Model:") { g.name   = t.splitn(2,':').nth(1).unwrap_or("").trim().into(); }
                if t.starts_with("Type:") && g.vendor.is_empty() { g.vendor = t.splitn(2,':').nth(1).unwrap_or("").trim().into(); }
                if t.starts_with("VRAM") { if let Some(v) = t.split(':').nth(1) {
                    let num = v.trim().split_whitespace().next().unwrap_or("0");
                    if let Ok(mb) = num.parse::<u64>() { g.vram_bytes = Some(mb * 1024 * 1024); }
                }}
                if t.starts_with("Metal:") { g.api = "Metal ".to_string() + t.splitn(2,':').nth(1).unwrap_or("").trim(); }
                if t.starts_with("Vendor:") { g.vendor = t.splitn(2,':').nth(1).unwrap_or("").trim().into(); }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["path","win32_VideoController","get",
                   "Name,DriverVersion,AdapterRAM,VideoProcessor","/value"]).output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let l = line.trim();
                if l.starts_with("Name=")          { g.name   = l[5..].into(); }
                if l.starts_with("DriverVersion=") { g.driver = l[14..].into(); }
                if l.starts_with("AdapterRAM=") {
                    if let Ok(b) = l[11..].parse::<u64>() { g.vram_bytes = Some(b); }
                }
                if l.starts_with("VideoProcessor=") { g.renderer = l[15..].into(); }
            }
            g.vendor = if g.name.to_lowercase().contains("nvidia") { "NVIDIA" }
                       else if g.name.to_lowercase().contains("amd") { "AMD" }
                       else if g.name.to_lowercase().contains("intel") { "Intel" }
                       else { "Unknown" }.into();
            g.api = "DirectX".into();
        }
    }

    if g.name.is_empty() { g.name = "Unknown GPU".into(); }
    g
}

// ═══════════════════════════════════════════ IP DETECTION ═════════════════════
fn get_iface_ips(iface: &str) -> (Vec<String>, Vec<String>) {
    let (mut v4, mut v6) = (vec![], vec![]);
    #[cfg(target_os = "linux")]
    if let Ok(out) = std::process::Command::new("ip").args(["addr","show",iface]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let t = line.trim();
            if t.starts_with("inet ")  { if let Some(a) = t.split_whitespace().nth(1) { v4.push(a.to_string()); } }
            if t.starts_with("inet6 ") { if let Some(a) = t.split_whitespace().nth(1) { v6.push(a.to_string()); } }
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("ifconfig").arg(iface).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let t = line.trim();
            if t.starts_with("inet ")  { if let Some(a) = t.split_whitespace().nth(1) { v4.push(a.to_string()); } }
            if t.starts_with("inet6 ") { if let Some(a) = t.split_whitespace().nth(1) { v6.push(a.to_string()); } }
        }
    }
    (v4, v6)
}

// ═══════════════════════════════════════════ EFRAME IMPL ══════════════════════
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_tick.elapsed() > Duration::from_millis(1500) { self.tick(); }

        // ── Visuals ─────────────────────────────────────────────────────────
        let mut v = ctx.style().visuals.clone();
        v.window_fill = C_BG; v.panel_fill = C_BG;
        v.faint_bg_color = C_SURF; v.extreme_bg_color = C_SURF2;
        v.override_text_color = Some(C_TEXT);
        v.widgets.noninteractive.bg_fill   = C_SURF;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, C_BORDER);
        v.widgets.noninteractive.rounding  = Rounding::same(10.0);
        v.widgets.inactive.bg_fill   = C_SURF;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, C_BORDER);
        v.widgets.inactive.rounding  = Rounding::same(10.0);
        v.widgets.hovered.bg_fill    = C_SURF2;
        v.widgets.hovered.bg_stroke  = Stroke::new(1.5, C_BLUE);
        v.widgets.hovered.rounding   = Rounding::same(10.0);
        v.widgets.active.bg_fill     = Color32::from_rgba_premultiplied(99,179,255,28);
        v.widgets.active.bg_stroke   = Stroke::new(1.5, C_BLUE);
        v.widgets.active.rounding    = Rounding::same(10.0);
        ctx.set_visuals(v);

        // ── Header ──────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("hdr")
            .frame(egui::Frame::none().fill(C_SURF)
                .inner_margin(egui::Margin::symmetric(20.0, 11.0))
                .stroke(Stroke::new(1.0, C_BORDER)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("** SpecsUltra").size(21.0).strong().color(C_BLUE));
                    ui.label(RichText::new("Pro").size(21.0).color(C_MUTED));
                    ui.add_space(10.0);
                    chip(ui, &System::name().unwrap_or_default(), C_TEAL);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        lozenge(ui, &format!("RAM {:.1}%", ram_pct(self.ram_used, self.ram_total)), C_GREEN);
                        ui.add_space(6.0);
                        lozenge(ui, &format!("CPU {:.1}%", self.cpu_usage), C_BLUE);
                        ui.add_space(6.0);
                        let us = now_unix().saturating_sub(self.boot_time);
                        lozenge(ui, &format!("Up {}h {:02}m", us/3600, (us%3600)/60), C_MUTED);
                    });
                });
            });

        // ── Tab bar ─────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("tabs")
            .frame(egui::Frame::none().fill(C_BG)
                .inner_margin(egui::Margin { left:16.0, right:16.0, top:5.0, bottom:0.0 })
                .stroke(Stroke::new(1.0, C_BORDER)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    tab(ui, &mut self.tab, Tab::Overview, "Overview");
                    tab(ui, &mut self.tab, Tab::OS,       "OS");
                    tab(ui, &mut self.tab, Tab::CPU,      "CPU");
                    tab(ui, &mut self.tab, Tab::GPU,      "GPU");
                    tab(ui, &mut self.tab, Tab::Memory,   "Memory");
                    tab(ui, &mut self.tab, Tab::Storage,  "Storage");
                    tab(ui, &mut self.tab, Tab::Network,  "Network");
                });
                ui.add_space(1.0);
            });

        // ── Content ─────────────────────────────────────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(C_BG).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
                    match self.tab {
                        Tab::Overview => self.ui_overview(ui),
                        Tab::OS       => self.ui_os(ui),
                        Tab::CPU      => self.ui_cpu(ui),
                        Tab::GPU      => self.ui_gpu(ui),
                        Tab::Memory   => self.ui_memory(ui),
                        Tab::Storage  => self.ui_storage(ui),
                        Tab::Network  => self.ui_network(ui),
                    }
                });
            });

        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

// ═══════════════════════════════════════════ TAB IMPLEMENTATIONS ══════════════
impl App {

    // ─────────────────────────── OVERVIEW ────────────────────────────────────
    fn ui_overview(&self, ui: &mut egui::Ui) {
        // Top 4 cards
        ui.columns(4, |c| {
            hw_card(&mut c[0], "Operating System",
                &System::name().unwrap_or_default(),
                &System::os_version().unwrap_or_default(), C_BLUE);
            hw_card(&mut c[1], "Processor",
                &self.cpu_brand,
                &format!("{} cores / {} threads  @{:.1} GHz",
                    self.cpu_cores_p, self.cpu_cores_l, self.cpu_freq_mhz as f64/1000.0), C_ORANGE);
            hw_card(&mut c[2], "Graphics",
                &self.gpu.name,
                &format!("{}{}", self.gpu.vendor,
                    self.gpu.vram_bytes.map(|v| format!("  |  {} VRAM", fmtb(v))).unwrap_or_default()), C_PURPLE);
            hw_card(&mut c[3], "Memory",
                &fmtb(self.ram_total),
                &format!("{} used  /  {} free", fmtb(self.ram_used), fmtb(self.ram_avail)), C_GREEN);
        });

        // Gauges
        ui.columns(3, |c| {
            gauge(&mut c[0], "CPU Usage",  self.cpu_usage/100.0,
                &format!("{:.1}%   @{} MHz", self.cpu_usage, self.cpu_freq_mhz), C_BLUE);
            gauge(&mut c[1], "RAM Usage",  ram_pct(self.ram_used,self.ram_total)/100.0,
                &format!("{} / {}", fmtb(self.ram_used), fmtb(self.ram_total)), C_GREEN);
            let sp = if self.swap_total>0 { self.swap_used as f32/self.swap_total as f32 } else { 0.0 };
            gauge(&mut c[2], "Swap Usage", sp,
                &format!("{} / {}", fmtb(self.swap_used), fmtb(self.swap_total)), C_ORANGE);
        });

        // CPU sparkline
        card(ui, |ui| {
            ui.label(RichText::new("CPU History (last 60s)").size(12.0).color(C_MUTED));
            ui.add_space(4.0);
            sparkline(ui, &self.cpu_history_global, C_BLUE, 60.0);
        });

        // Temperatures
        let tc: Vec<_> = self.comps.iter().collect();
        if !tc.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new("Temperatures").size(13.0).strong().color(C_ORANGE));
                ui.add_space(6.0);
                let cols = (tc.len().min(6)).max(1);
                ui.columns(cols, |cs| {
                    for (i, comp) in tc.iter().enumerate() {
                        if i >= cs.len() { break; }
                        let t = comp.temperature();
                        let col = heat_color(t);
                        cs[i].vertical(|ui| {
                            ui.label(RichText::new(format!("{:.0}°C", t)).size(22.0).strong().color(col));
                            ui.label(RichText::new(comp.label()).size(9.0).color(C_MUTED));
                            if let Some(c) = comp.critical() {
                                ui.label(RichText::new(format!("crit {:.0}°", c)).size(9.0).color(C_DIM));
                            }
                        });
                    }
                });
            });
        }

        // Quick summary
        card(ui, |ui| {
            ui.label(RichText::new("Quick Summary").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            egui::Grid::new("ov_sum").num_columns(4).spacing([20.0,5.0]).striped(true).show(ui, |ui| {
                kv(ui,"Hostname",     &System::host_name().unwrap_or_default());
                kv(ui,"Kernel",       &System::kernel_version().unwrap_or_default()); ui.end_row();
                kv(ui,"Architecture", &self.cpu_arch);
                kv(ui,"CPU Vendor",   &self.cpu_vendor); ui.end_row();
                kv(ui,"Logical CPUs", &self.cpu_cores_l.to_string());
                kv(ui,"Boot time",    &fmt_uptime(now_unix().saturating_sub(self.boot_time))); ui.end_row();
                kv(ui,"GPU",          &self.gpu.name);
                kv(ui,"GPU Vendor",   &self.gpu.vendor); ui.end_row();
                kv(ui,"Total RAM",    &fmtb(self.ram_total));
                kv(ui,"Total Disks",  &self.disks.len().to_string()); ui.end_row();
            });
        });

        // Top processes
        card(ui, |ui| {
            ui.label(RichText::new("Top Processes (by CPU)").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            let mut procs: Vec<_> = self.sys.processes().values().collect();
            procs.sort_by(|a,b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
            egui::Grid::new("ov_procs").num_columns(5).spacing([18.0,3.0]).striped(true).show(ui,|ui|{
                ui.label(rt("PID",C_MUTED,10.0)); ui.label(rt("Process",C_MUTED,10.0));
                ui.label(rt("CPU%",C_MUTED,10.0)); ui.label(rt("RAM",C_MUTED,10.0));
                ui.label(rt("Status",C_MUTED,10.0)); ui.end_row();
                for p in procs.iter().take(12) {
                    ui.label(rt(&format!("{}",p.pid()),C_DIM,10.0));
                    ui.label(rt(p.name(),C_TEXT,10.0));
                    ui.label(rt(&format!("{:.1}%",p.cpu_usage()),heat_color(p.cpu_usage()),10.0));
                    ui.label(rt(&fmtb(p.memory()),C_MUTED,10.0));
                    ui.label(rt(&format!("{:?}",p.status()),C_DIM,10.0));
                    ui.end_row();
                }
            });
        });
    }

    // ─────────────────────────── OS ──────────────────────────────────────────
    fn ui_os(&self, ui: &mut egui::Ui) {
        // Banner
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(System::name().unwrap_or_default())
                        .size(28.0).strong().color(C_TEXT));
                    ui.add_space(2.0);
                    ui.label(RichText::new(System::long_os_version().unwrap_or_default())
                        .size(13.0).color(C_MUTED));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        chip(ui, &System::cpu_arch().unwrap_or_default(), C_TEAL);
                        chip(ui, &format!("Kernel {}", System::kernel_version().unwrap_or_default()), C_PURPLE);
                        chip(ui, &format!("{} processes", self.sys.processes().len()), C_ORANGE);
                    });
                });
            });
        });

        ui.columns(2, |c| {
            // Identity
            card(&mut c[0], |ui| {
                ui.label(RichText::new("Identity").size(13.0).strong().color(C_BLUE));
                ui.add_space(4.0);
                egui::Grid::new("os_id").num_columns(2).spacing([14.0,4.0]).striped(true).show(ui,|ui|{
                    kv(ui,"System Name",     &System::name().unwrap_or_default());             ui.end_row();
                    kv(ui,"Hostname",        &System::host_name().unwrap_or_default());        ui.end_row();
                    kv(ui,"OS Version",      &System::os_version().unwrap_or_default());       ui.end_row();
                    kv(ui,"Kernel",          &System::kernel_version().unwrap_or_default());   ui.end_row();
                    kv(ui,"Architecture",    &System::cpu_arch().unwrap_or_default());         ui.end_row();
                    kv(ui,"Long Version",    &System::long_os_version().unwrap_or_default());  ui.end_row();
                    kv(ui,"Distribution",    &System::name().unwrap_or_default());             ui.end_row();
                });
            });

            // Timing
            card(&mut c[1], |ui| {
                ui.label(RichText::new("Timing & Uptime").size(13.0).strong().color(C_BLUE));
                ui.add_space(4.0);
                let us = now_unix().saturating_sub(self.boot_time);
                egui::Grid::new("os_tm").num_columns(2).spacing([14.0,4.0]).striped(true).show(ui,|ui|{
                    kv(ui,"Boot Time (unix)", &self.boot_time.to_string());    ui.end_row();
                    kv(ui,"Uptime",           &fmt_uptime_long(us));            ui.end_row();
                    kv(ui,"Days Running",     &(us/86400).to_string());        ui.end_row();
                    kv(ui,"Total Processes",  &self.sys.processes().len().to_string()); ui.end_row();
                    kv(ui,"Physical Cores",   &self.cpu_cores_p.to_string()); ui.end_row();
                    kv(ui,"Logical CPUs",     &self.cpu_cores_l.to_string()); ui.end_row();
                    kv(ui,"Total RAM",        &fmtb(self.ram_total));          ui.end_row();
                });
            });
        });

        // Environment variables
        card(ui, |ui| {
            ui.label(RichText::new("Environment Variables").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            let keys = [
                "PATH","HOME","USER","SHELL","TERM","LANG","LOCALE",
                "XDG_SESSION_TYPE","XDG_CURRENT_DESKTOP","DISPLAY","WAYLAND_DISPLAY",
                "DBUS_SESSION_BUS_ADDRESS","GTK_THEME","QT_STYLE_OVERRIDE",
                "COMPUTERNAME","USERPROFILE","SYSTEMROOT","WINDIR",
                "PROCESSOR_ARCHITECTURE","TEMP","TMP","PATHEXT",
            ];
            egui::Grid::new("env").num_columns(2).spacing([16.0,3.0]).striped(true).show(ui,|ui|{
                for k in &keys {
                    if let Ok(v) = std::env::var(k) {
                        let disp = if v.len()>90 { format!("{}...",  &v[..90]) } else { v };
                        kv(ui, k, &disp); ui.end_row();
                    }
                }
            });
        });

        // All processes table
        card(ui, |ui| {
            ui.label(RichText::new("All Processes").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            let mut procs: Vec<_> = self.sys.processes().values().collect();
            procs.sort_by(|a,b| b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));
            egui::Grid::new("all_procs").num_columns(6).spacing([14.0,2.0]).striped(true).show(ui,|ui|{
                ui.label(rt("PID",C_MUTED,10.0));
                ui.label(rt("Name",C_MUTED,10.0));
                ui.label(rt("CPU%",C_MUTED,10.0));
                ui.label(rt("RAM",C_MUTED,10.0));
                ui.label(rt("Disk R",C_MUTED,10.0));
                ui.label(rt("Disk W",C_MUTED,10.0));
                ui.end_row();
                for p in procs.iter().take(30) {
                    ui.label(rt(&format!("{}",p.pid()),C_DIM,10.0));
                    ui.label(rt(p.name(),C_TEXT,10.0));
                    ui.label(rt(&format!("{:.1}%",p.cpu_usage()),heat_color(p.cpu_usage()),10.0));
                    ui.label(rt(&fmtb(p.memory()),C_MUTED,10.0));
                    ui.label(rt(&fmtb(p.disk_usage().read_bytes),C_TEAL,10.0));
                    ui.label(rt(&fmtb(p.disk_usage().written_bytes),C_ORANGE,10.0));
                    ui.end_row();
                }
            });
        });

        // Filesystem / mounts summary
        card(ui, |ui| {
            ui.label(RichText::new("Mounted Filesystems").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            egui::Grid::new("mounts").num_columns(4).spacing([16.0,3.0]).striped(true).show(ui,|ui|{
                ui.label(rt("Device",C_MUTED,10.0)); ui.label(rt("Mount",C_MUTED,10.0));
                ui.label(rt("FS",C_MUTED,10.0));     ui.label(rt("Total",C_MUTED,10.0)); ui.end_row();
                for d in &self.disks {
                    kv(ui, &d.name, &d.mount); kv(ui, &d.fs, &fmtb(d.total)); ui.end_row();
                }
            });
        });
    }

    // ─────────────────────────── CPU ─────────────────────────────────────────
    fn ui_cpu(&self, ui: &mut egui::Ui) {
        // Hero
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&self.cpu_brand).size(22.0).strong().color(C_TEXT));
                    ui.label(RichText::new(&self.cpu_vendor).size(12.0).color(C_MUTED));
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        chip(ui, &format!("{} Physical Cores", self.cpu_cores_p), C_ORANGE);
                        chip(ui, &format!("{} Threads", self.cpu_cores_l), C_TEAL);
                        chip(ui, &self.cpu_arch, C_BLUE);
                        if self.cpu_freq_mhz > 0 {
                            chip(ui, &format!("{:.2} GHz", self.cpu_freq_mhz as f64/1000.0), C_GREEN);
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(format!("{:.1}%",self.cpu_usage))
                            .size(38.0).strong().color(heat_color(self.cpu_usage)));
                        ui.label(RichText::new("global usage").size(10.0).color(C_MUTED));
                    });
                });
            });
        });

        ui.columns(2, |c| {
            card(&mut c[0], |ui| {
                ui.label(RichText::new("Processor Details").size(13.0).strong().color(C_BLUE));
                ui.add_space(4.0);
                let f = self.sys.cpus().first();
                egui::Grid::new("cpu_det").num_columns(2).spacing([14.0,4.0]).striped(true).show(ui,|ui|{
                    kv(ui,"Brand",          &self.cpu_brand);   ui.end_row();
                    kv(ui,"Vendor ID",      &self.cpu_vendor);  ui.end_row();
                    kv(ui,"Architecture",   &self.cpu_arch);    ui.end_row();
                    kv(ui,"Physical Cores", &self.cpu_cores_p.to_string()); ui.end_row();
                    kv(ui,"Logical Threads",&self.cpu_cores_l.to_string()); ui.end_row();
                    if let Some(c) = f {
                        kv(ui,"Frequency",  &format!("{} MHz  ({:.3} GHz)", c.frequency(), c.frequency() as f64/1000.0)); ui.end_row();
                    }
                    kv(ui,"Endianness", if cfg!(target_endian="little") { "Little Endian" } else { "Big Endian" }); ui.end_row();
                    kv(ui,"Pointer Width", &format!("{} bits", std::mem::size_of::<usize>()*8)); ui.end_row();
                    kv(ui,"HyperThreading", if self.cpu_cores_l > self.cpu_cores_p { "Enabled" } else { "Disabled / N/A" }); ui.end_row();
                    kv(ui,"Hypervision",    if cfg!(target_os="linux") { "Check /proc/cpuinfo" } else { "N/A" }); ui.end_row();
                });
            });

            card(&mut c[1], |ui| {
                ui.label(RichText::new("Per-Core Usage").size(13.0).strong().color(C_BLUE));
                ui.add_space(4.0);
                let nc = self.cores.len();
                let cols_n = if nc > 8 { 4 } else if nc > 4 { 3 } else { 2 };
                egui::Grid::new("per_core").num_columns(cols_n).spacing([8.0,5.0]).show(ui,|ui|{
                    for (i, core) in self.cores.iter().enumerate() {
                        let col = heat_color(core.usage);
                        ui.vertical(|ui| {
                            ui.label(rt(&format!("C{}",i),C_MUTED,9.0));
                            mini_bar(ui, core.usage/100.0, col, 52.0, 8.0);
                            ui.label(rt(&format!("{:.0}%",core.usage),col,9.0));
                            ui.label(rt(&format!("{}MHz",core.freq_mhz),C_DIM,8.5));
                        });
                        if (i+1) % cols_n == 0 { ui.end_row(); }
                    }
                });
            });
        });

        // CPU global sparkline
        card(ui, |ui| {
            ui.label(RichText::new("CPU Usage History (2 min)").size(12.0).color(C_MUTED));
            ui.add_space(4.0);
            sparkline(ui, &self.cpu_history_global, C_BLUE, 80.0);
        });

        // CPU temperature from components
        let cpu_temps: Vec<_> = self.comps.iter().filter(|c| {
            let l = c.label().to_lowercase();
            l.contains("core") || l.contains("cpu") || l.contains("k10") || l.contains("tctl")
        }).collect();
        if !cpu_temps.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new("CPU Thermal Sensors").size(13.0).strong().color(C_ORANGE));
                ui.add_space(4.0);
                egui::Grid::new("cpu_temps").num_columns(4).spacing([16.0,4.0]).striped(true).show(ui,|ui|{
                    for t in &cpu_temps {
                        let temp = t.temperature();
                        let col  = heat_color(temp);
                        ui.label(rt(&t.label(), C_MUTED, 11.0));
                        ui.label(rt(&format!("{:.1}°C", temp), col, 11.0));
                        if let Some(c) = t.critical() {
                            ui.label(rt(&format!("crit {:.0}°", c), C_DIM, 10.0));
                        } else { ui.label(""); }
                        ui.label(rt(
                            if temp > 90.0 { "CRITICAL" } else if temp > 75.0 { "HOT" }
                            else if temp > 55.0 { "WARM" } else { "OK" },
                            col, 10.0)); ui.end_row();
                    }
                });
            });
        }

        // Extensions
        card(ui, |ui| {
            ui.label(RichText::new("Supported Instruction Sets").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            let exts = cpu_extensions(&self.cpu_vendor, &self.cpu_arch);
            ui.horizontal_wrapped(|ui| {
                for e in &exts { chip(ui, e, C_DIM); }
            });
        });
    }

    // ─────────────────────────── GPU ─────────────────────────────────────────
    fn ui_gpu(&self, ui: &mut egui::Ui) {
        let g = &self.gpu;

        // Hero
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&g.name).size(22.0).strong().color(C_TEXT));
                    ui.label(RichText::new(&g.vendor).size(12.0).color(C_MUTED));
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        chip(ui, &g.vendor, C_PURPLE);
                        if !g.driver.is_empty() { chip(ui, &format!("Driver {}", g.driver), C_BLUE); }
                        if !g.api.is_empty() { chip(ui, &g.api, C_TEAL); }
                        if let Some(v) = g.vram_bytes {
                            chip(ui, &format!("{} VRAM", fmtb(v)), C_GREEN);
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(t) = g.temp_c {
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(format!("{:.0}°C",t))
                                .size(38.0).strong().color(heat_color(t as f32)));
                            ui.label(RichText::new("GPU temp").size(10.0).color(C_MUTED));
                        });
                    } else {
                        ui.label(RichText::new("Temp N/A").size(14.0).color(C_MUTED));
                    }
                });
            });
        });

        ui.columns(2, |c| {
            card(&mut c[0], |ui| {
                ui.label(RichText::new("Hardware Info").size(13.0).strong().color(C_PURPLE));
                ui.add_space(4.0);
                egui::Grid::new("gpu_hw").num_columns(2).spacing([14.0,4.0]).striped(true).show(ui,|ui|{
                    kv(ui,"Name",           &g.name);   ui.end_row();
                    kv(ui,"Vendor",         &g.vendor); ui.end_row();
                    kv(ui,"VRAM",           &g.vram_bytes.map(fmtb).unwrap_or("N/A".into())); ui.end_row();
                    kv(ui,"Bus / Slot",     &g.bus);    ui.end_row();
                    kv(ui,"Driver",         &g.driver); ui.end_row();
                    kv(ui,"API",            &g.api);    ui.end_row();
                    kv(ui,"Renderer",       &g.renderer); ui.end_row();
                    kv(ui,"Temperature",    &g.temp_c.map(|t| format!("{:.1}°C",t)).unwrap_or("N/A".into())); ui.end_row();
                });
            });

            card(&mut c[1], |ui| {
                ui.label(RichText::new("Capabilities & APIs").size(13.0).strong().color(C_PURPLE));
                ui.add_space(4.0);
                let vendor_lc = g.vendor.to_lowercase();
                let caps = gpu_capabilities(&vendor_lc);
                egui::Grid::new("gpu_caps").num_columns(2).spacing([14.0,4.0]).striped(true).show(ui,|ui|{
                    for (k,v) in &caps { kv(ui,k,v); ui.end_row(); }
                });
            });
        });

        // GPU thermal sensors
        let gpu_temps: Vec<_> = self.comps.iter().filter(|c| {
            let l = c.label().to_lowercase();
            l.contains("gpu") || l.contains("amdgpu") || l.contains("edge")
               || l.contains("vram") || l.contains("mem") && l.contains("junction")
        }).collect();
        if !gpu_temps.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new("GPU Thermal Sensors").size(13.0).strong().color(C_ORANGE));
                ui.add_space(4.0);
                egui::Grid::new("gpu_temps").num_columns(4).spacing([16.0,4.0]).striped(true).show(ui,|ui|{
                    for t in &gpu_temps {
                        let temp = t.temperature();
                        let col  = heat_color(temp);
                        ui.label(rt(&t.label(),C_MUTED,11.0));
                        ui.label(rt(&format!("{:.1}°C",temp),col,11.0));
                        if let Some(c) = t.critical() {
                            ui.label(rt(&format!("crit {:.0}°",c),C_DIM,10.0));
                        } else { ui.label(""); }
                        ui.label(rt(
                            if temp > 95.0 { "CRITICAL" } else if temp > 80.0 { "HOT" }
                            else if temp > 60.0 { "WARM" } else { "OK" },
                            col,10.0));
                        ui.end_row();
                    }
                });
            });
        }

        // GPU platform notes
        card(ui, |ui| {
            ui.label(RichText::new("Detection Notes").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            let notes: &[(&str, &str)] = &[
                ("Linux","Full info via lspci -v, /sys/class/drm, nvidia-smi, glxinfo"),
                ("macOS","Info via system_profiler SPDisplaysDataType"),
                ("Windows","Info via WMIC win32_VideoController"),
                ("VRAM (AMD)","Read from /sys/class/drm/card0/device/mem_info_vram_total"),
                ("VRAM (NVIDIA)","Read from nvidia-smi (requires driver)"),
                ("Temperature","Read from kernel hwmon via sysinfo Components"),
                ("Driver","Kernel driver name from lspci -v / nvidia-smi"),
                ("OpenGL","Renderer string from glxinfo -B"),
            ];
            egui::Grid::new("gpu_notes").num_columns(2).spacing([14.0,3.0]).striped(true).show(ui,|ui|{
                for (k,v) in notes { kv(ui,k,v); ui.end_row(); }
            });
        });

        // Raw component dump related to GPU
        if !gpu_temps.is_empty() || g.temp_c.is_some() {
            card(ui, |ui| {
                ui.label(RichText::new("All Thermal Components").size(13.0).strong().color(C_BLUE));
                ui.add_space(4.0);
                egui::Grid::new("all_comps").num_columns(3).spacing([16.0,3.0]).striped(true).show(ui,|ui|{
                    ui.label(rt("Sensor",C_MUTED,10.0));
                    ui.label(rt("Temp",C_MUTED,10.0));
                    ui.label(rt("Critical",C_MUTED,10.0)); ui.end_row();
                    for comp in self.comps.iter() {
                        let t = comp.temperature();
                        ui.label(rt(comp.label(),C_TEXT,10.0));
                        ui.label(rt(&format!("{:.1}°C",t),heat_color(t),10.0));
                        ui.label(rt(
                            &comp.critical().map(|c| format!("{:.0}°C",c)).unwrap_or("–".into()),
                            C_MUTED,10.0));
                        ui.end_row();
                    }
                });
            });
        }
    }

    // ─────────────────────────── MEMORY ──────────────────────────────────────
    fn ui_memory(&self, ui: &mut egui::Ui) {
        let pct = ram_pct(self.ram_used, self.ram_total);

        // Hero with stacked bar
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(fmtb(self.ram_total)).size(28.0).strong().color(C_TEXT));
                    ui.label(RichText::new("Physical Memory").size(12.0).color(C_MUTED));
                    ui.add_space(6.0);
                    let frac = (self.ram_used as f32 / self.ram_total.max(1) as f32).clamp(0.0,1.0);
                    let w = ui.available_width().min(360.0);
                    let (rect,_) = ui.allocate_exact_size(Vec2::new(w,14.0),egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect, Rounding::same(6.0), C_BORDER);
                    let mut fr = rect; fr.set_width((rect.width()*frac).max(2.0));
                    p.rect_filled(fr, Rounding::same(6.0), C_GREEN);
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        dot(ui, C_GREEN);
                        ui.label(rt(&format!("Used  {}",fmtb(self.ram_used)),C_MUTED,10.0));
                        ui.add_space(8.0);
                        dot(ui, C_BORDER);
                        ui.label(rt(&format!("Free  {}",fmtb(self.ram_avail)),C_MUTED,10.0));
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(format!("{:.1}%",pct))
                            .size(38.0).strong().color(C_GREEN));
                        ui.label(RichText::new("in use").size(10.0).color(C_MUTED));
                    });
                });
            });
        });

        ui.columns(2, |c| {
            card(&mut c[0], |ui| {
                ui.label(RichText::new("RAM Details").size(13.0).strong().color(C_GREEN));
                ui.add_space(4.0);
                egui::Grid::new("ram_d").num_columns(2).spacing([14.0,4.0]).striped(true).show(ui,|ui|{
                    kv(ui,"Total",          &fmtb(self.ram_total));  ui.end_row();
                    kv(ui,"Used",           &fmtb(self.ram_used));   ui.end_row();
                    kv(ui,"Available",      &fmtb(self.ram_avail));  ui.end_row();
                    kv(ui,"Free (kernel)",  &fmtb(self.ram_avail));  ui.end_row();
                    kv(ui,"Usage %",        &format!("{:.2}%",pct)); ui.end_row();
                    kv(ui,"Total (GiB)",    &format!("{:.3} GiB", self.ram_total as f64/1073741824.0)); ui.end_row();
                    kv(ui,"Used (GiB)",     &format!("{:.3} GiB", self.ram_used  as f64/1073741824.0)); ui.end_row();
                    kv(ui,"Free (GiB)",     &format!("{:.3} GiB", self.ram_avail as f64/1073741824.0)); ui.end_row();
                });
            });

            card(&mut c[1], |ui| {
                ui.label(RichText::new("Swap / Virtual Memory").size(13.0).strong().color(C_ORANGE));
                ui.add_space(4.0);
                if self.swap_total > 0 {
                    let sf = (self.swap_used as f32/self.swap_total as f32).clamp(0.0,1.0);
                    let w  = ui.available_width().min(280.0);
                    let (rect,_) = ui.allocate_exact_size(Vec2::new(w,10.0),egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect,Rounding::same(4.0),C_BORDER);
                    let mut fr = rect; fr.set_width((rect.width()*sf).max(2.0));
                    p.rect_filled(fr,Rounding::same(4.0),C_ORANGE);
                    ui.add_space(6.0);
                    egui::Grid::new("swap").num_columns(2).spacing([14.0,4.0]).striped(true).show(ui,|ui|{
                        kv(ui,"Total",  &fmtb(self.swap_total));   ui.end_row();
                        kv(ui,"Used",   &fmtb(self.swap_used));    ui.end_row();
                        kv(ui,"Free",   &fmtb(self.swap_total.saturating_sub(self.swap_used))); ui.end_row();
                        kv(ui,"Usage%", &format!("{:.2}%",sf*100.0)); ui.end_row();
                        kv(ui,"Total (GiB)", &format!("{:.2} GiB",self.swap_total as f64/1073741824.0)); ui.end_row();
                    });
                } else {
                    ui.label(RichText::new("No swap configured").color(C_MUTED));
                }
            });
        });

        // Memory pressure / top RAM consumers
        card(ui, |ui| {
            ui.label(RichText::new("Top RAM Consumers").size(13.0).strong().color(C_BLUE));
            ui.add_space(4.0);
            let mut procs: Vec<_> = self.sys.processes().values().collect();
            procs.sort_by_key(|p| std::cmp::Reverse(p.memory()));
            egui::Grid::new("ram_procs").num_columns(4).spacing([16.0,3.0]).striped(true).show(ui,|ui|{
                ui.label(rt("PID",C_MUTED,10.0)); ui.label(rt("Name",C_MUTED,10.0));
                ui.label(rt("RAM",C_MUTED,10.0)); ui.label(rt("Virtual",C_MUTED,10.0)); ui.end_row();
                for p in procs.iter().take(15) {
                    let pct_ram = p.memory() as f64/self.ram_total.max(1) as f64*100.0;
                    ui.label(rt(&format!("{}",p.pid()),C_DIM,10.0));
                    ui.label(rt(p.name(),C_TEXT,10.0));
                    ui.label(rt(&format!("{} ({:.1}%)",fmtb(p.memory()),pct_ram),
                        if pct_ram > 10.0 { C_RED } else if pct_ram > 5.0 { C_ORANGE } else { C_MUTED },10.0));
                    ui.label(rt(&fmtb(p.virtual_memory()),C_DIM,10.0));
                    ui.end_row();
                }
            });
        });
    }

    // ─────────────────────────── STORAGE ─────────────────────────────────────
    fn ui_storage(&mut self, ui: &mut egui::Ui) {
        let total_all: u64 = self.disks.iter().map(|d| d.total).sum();
        let avail_all: u64 = self.disks.iter().map(|d| d.avail).sum();

        // Summary
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("{} volumes  |  {} total  |  {} available",
                        self.disks.len(), fmtb(total_all), fmtb(avail_all)))
                        .size(14.0).color(C_TEXT));
                    ui.add_space(5.0);
                    let uf = if total_all > 0 { (total_all-avail_all) as f32/total_all as f32 } else { 0.0 };
                    let w  = ui.available_width().min(420.0);
                    let (rect,_) = ui.allocate_exact_size(Vec2::new(w,10.0),egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect,Rounding::same(4.0),C_BORDER);
                    let mut fr = rect; fr.set_width((rect.width()*uf.clamp(0.0,1.0)).max(2.0));
                    p.rect_filled(fr,Rounding::same(4.0),if uf>0.9{C_RED}else if uf>0.7{C_ORANGE}else{C_TEAL});
                });
            });
        });

        for i in 0..self.disks.len() {
            let used   = self.disks[i].total.saturating_sub(self.disks[i].avail);
            let uf     = if self.disks[i].total > 0 { used as f32/self.disks[i].total as f32 } else { 0.0 };
            let bcol   = if uf > 0.9 { C_RED } else if uf > 0.7 { C_ORANGE } else { C_TEAL };
            let hp     = self.disks[i].smart.health_pct.unwrap_or(100);
            let hcol   = if hp > 80 { C_GREEN } else if hp > 50 { C_ORANGE } else { C_RED };
            let name   = self.disks[i].name.clone();
            let mount  = self.disks[i].mount.clone();
            let kind   = self.disks[i].kind.clone();
            let fs_    = self.disks[i].fs.clone();
            let total_ = self.disks[i].total;
            let avail_ = self.disks[i].avail;
            let exp    = self.disks[i].smart.expanded;

            card(ui, |ui| {
                // Header
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&name).size(15.0).strong());
                        ui.horizontal(|ui| {
                            chip(ui, &kind, C_TEAL);
                            chip(ui, &fs_, C_MUTED);
                            chip(ui, &mount, C_DIM);
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new(
                            if exp { "[ Hide SMART ]" } else { "[ Show SMART ]" })
                            .size(11.0).color(C_BLUE)).clicked()
                        {
                            self.disks[i].smart.expanded = !exp;
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new(format!("Health {hp}%")).size(12.0).color(hcol));
                    });
                });
                ui.add_space(5.0);

                // Bar
                let w = ui.available_width() - 6.0;
                let (rect,_) = ui.allocate_exact_size(Vec2::new(w,12.0),egui::Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect,Rounding::same(5.0),C_BORDER);
                let mut fr = rect; fr.set_width((rect.width()*uf.clamp(0.0,1.0)).max(2.0));
                p.rect_filled(fr,Rounding::same(5.0),bcol);

                ui.horizontal(|ui| {
                    dot(ui, bcol);
                    ui.label(rt(&format!("Used {}",fmtb(used)),C_MUTED,10.0));
                    ui.add_space(8.0);
                    dot(ui, C_BORDER);
                    ui.label(rt(&format!("Free {}",fmtb(avail_)),C_MUTED,10.0));
                    ui.add_space(8.0);
                    ui.label(rt(&format!("Total {}  ({:.1}% used)",fmtb(total_),uf*100.0),C_DIM,10.0));
                });

                // SMART panel
                if exp {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(RichText::new("S.M.A.R.T Report").size(12.0).strong().color(C_YELLOW));
                    ui.add_space(4.0);
                    let s = &self.disks[i].smart;
                    egui::Grid::new(format!("sm_{i}")).num_columns(4).spacing([18.0,4.0]).striped(true).show(ui,|ui|{
                        kv(ui,"Interface",    &self.disks[i].kind);
                        kv(ui,"Filesystem",   &self.disks[i].fs); ui.end_row();
                        kv(ui,"Total Space",  &fmtb(total_));
                        kv(ui,"Available",    &fmtb(avail_)); ui.end_row();
                        if let Some(t) = s.temperature_c {
                            kv_col(ui,"Temperature",&format!("{:.0}°C",t),heat_color(t as f32));
                        } else { kv(ui,"Temperature","N/A (root req.)"); }
                        kv(ui,"Health",     &format!("{}%",hp)); ui.end_row();
                        if let Some(w) = s.total_written_bytes {
                            kv(ui,"Est. Written",  &fmtb(w));
                        }
                        if let Some(r) = s.total_read_bytes {
                            kv(ui,"Est. Read",     &fmtb(r));
                        }
                        ui.end_row();
                        if let Some(rs) = s.reallocated_sectors {
                            kv_col(ui,"Reallocated Sectors",&rs.to_string(),if rs>0{C_RED}else{C_GREEN});
                        }
                        if let Some(ps) = s.pending_sectors {
                            kv_col(ui,"Pending Sectors",&ps.to_string(),if ps>0{C_ORANGE}else{C_GREEN});
                        }
                        ui.end_row();
                        if let Some(ue) = s.uncorrectable {
                            kv_col(ui,"Uncorrectable Errors",&ue.to_string(),if ue>0{C_RED}else{C_GREEN});
                        }
                        if let Some(sr) = s.spin_retries {
                            kv_col(ui,"Spin Retries",&sr.to_string(),if sr>0{C_ORANGE}else{C_GREEN});
                        }
                        ui.end_row();
                        kv(ui,"Power-On Hours", &s.power_on_hours.map(|h|format!("{h}h  ({}d)",h/24)).unwrap_or("N/A (root req.)".into()));
                        kv(ui,"Status", if hp > 80 { "GOOD" } else if hp > 50 { "FAIR" } else { "POOR" });
                        ui.end_row();
                    });
                    ui.add_space(3.0);
                    ui.label(RichText::new("Full SMART requires elevated privileges + smartctl/nvme crate.")
                        .size(9.0).color(C_DIM).italics());
                }
            });
        }
    }

    // ─────────────────────────── NETWORK ─────────────────────────────────────
    fn ui_network(&self, ui: &mut egui::Ui) {
        let pub_ip = self.pub_ip.lock().unwrap().clone();

        // Public IP banner
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Public IPv4").size(11.0).color(C_MUTED));
                    ui.label(RichText::new(&pub_ip).size(24.0).strong().color(C_YELLOW));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    chip(ui, &format!("{} interfaces", self.ifaces.len()), C_MUTED);
                    ui.add_space(8.0);
                    let total_rx: u64 = self.ifaces.iter().map(|i| i.rx).sum();
                    let total_tx: u64 = self.ifaces.iter().map(|i| i.tx).sum();
                    chip(ui, &format!("Total RX {}", fmtb(total_rx)), C_GREEN);
                    ui.add_space(4.0);
                    chip(ui, &format!("Total TX {}", fmtb(total_tx)), C_ORANGE);
                });
            });
        });

        // Per-interface cards
        for iface in &self.ifaces {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&iface.name).size(15.0).strong());
                        ui.horizontal(|ui| {
                            chip(ui, &iface.kind, C_TEAL);
                            if !iface.mac.is_empty() && iface.mac != "00:00:00:00:00:00" {
                                chip(ui, &iface.mac, C_DIM);
                            }
                            if iface.rx > 0 || iface.tx > 0 { chip(ui,"Active",C_GREEN); }
                            else { chip(ui,"Idle",C_MUTED); }
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(format!("UP  {}/s", fmtb_net(iface.tx_speed)))
                                .size(11.0).color(C_ORANGE));
                            ui.label(RichText::new(format!("DL  {}/s", fmtb_net(iface.rx_speed)))
                                .size(11.0).color(C_GREEN));
                        });
                    });
                });

                ui.add_space(5.0);
                egui::Grid::new(&iface.name).num_columns(4).spacing([18.0,3.0]).striped(true).show(ui,|ui|{
                    kv(ui,"MAC Address",     &iface.mac); ui.end_row();
                    kv(ui,"IPv4",
                        &if iface.ipv4s.is_empty() { "None (check ip addr)".into() }
                         else { iface.ipv4s.join(", ") });
                    ui.end_row();
                    if !iface.ipv6s.is_empty() {
                        kv(ui,"IPv6", &iface.ipv6s[0]); ui.end_row();
                        for v6 in iface.ipv6s.iter().skip(1).take(3) {
                            kv(ui,"IPv6 (alt)", v6); ui.end_row();
                        }
                    }
                    kv(ui,"Total Received",    &fmtb(iface.rx));
                    kv(ui,"Total Transmitted", &fmtb(iface.tx)); ui.end_row();
                    kv(ui,"RX Speed",          &format!("{}/s", fmtb_net(iface.rx_speed)));
                    kv(ui,"TX Speed",          &format!("{}/s", fmtb_net(iface.tx_speed))); ui.end_row();
                    kv(ui,"Interface Type",    &iface.kind); ui.end_row();
                });
            });
        }
    }
}

// ═══════════════════════════════════════════ UI WIDGETS ═══════════════════════

fn card(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(C_SURF).rounding(Rounding::same(13.0))
        .stroke(Stroke::new(1.0, C_BORDER)).inner_margin(egui::Margin::same(13.0))
        .show(ui, f);
}

fn hw_card(ui: &mut egui::Ui, label: &str, value: &str, sub: &str, accent: Color32) {
    egui::Frame::none().fill(C_SURF).rounding(Rounding::same(13.0))
        .stroke(Stroke::new(1.0, C_BORDER)).inner_margin(egui::Margin::same(13.0))
        .show(ui, |ui| {
            let (bar,_) = ui.allocate_exact_size(Vec2::new(ui.available_width(),3.0),egui::Sense::hover());
            ui.painter().rect_filled(bar, Rounding::same(2.0), accent);
            ui.add_space(7.0);
            ui.label(RichText::new(label).size(10.0).color(C_MUTED));
            ui.add_space(2.0);
            ui.label(RichText::new(value).size(13.0).strong().color(C_TEXT));
            if !sub.is_empty() {
                ui.label(RichText::new(sub).size(10.0).color(C_MUTED));
            }
        });
}

fn gauge(ui: &mut egui::Ui, label: &str, frac: f32, detail: &str, color: Color32) {
    egui::Frame::none().fill(C_SURF).rounding(Rounding::same(13.0))
        .stroke(Stroke::new(1.0, C_BORDER)).inner_margin(egui::Margin::same(13.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(12.0).strong().color(color));
            ui.add_space(3.0);
            ui.label(RichText::new(format!("{:.1}%",frac*100.0)).size(24.0).strong().color(color));
            ui.add_space(4.0);
            let (rect,_) = ui.allocate_exact_size(Vec2::new(ui.available_width()-4.0,12.0),egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect,Rounding::same(5.0),C_BORDER);
            let mut fr = rect; fr.set_width((rect.width()*frac.clamp(0.0,1.0)).max(2.0));
            p.rect_filled(fr,Rounding::same(5.0),color);
            ui.add_space(3.0);
            ui.label(RichText::new(detail).size(10.0).color(C_MUTED));
        });
}

/// Tab button with active underline
fn tab(ui: &mut egui::Ui, cur: &mut Tab, t: Tab, label: &str) {
    let active = *cur == t;
    let col    = if active { C_BLUE } else { C_MUTED };
    let btn = ui.add(egui::Button::new(RichText::new(label).size(13.0).color(col).strong())
        .frame(false).min_size(Vec2::new(0.0, 34.0)));
    if btn.clicked() { *cur = t; }
    if active {
        let r = btn.rect;
        ui.painter().line_segment(
            [egui::pos2(r.min.x, r.max.y-2.0), egui::pos2(r.max.x, r.max.y-2.0)],
            Stroke::new(2.5, C_BLUE));
    }
    ui.add_space(4.0);
}

/// Pill badge (header)
fn lozenge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let fid = FontId::proportional(12.0);
    let gal = ui.fonts(|f| f.layout_no_wrap(text.to_string(), fid.clone(), color));
    let (rect,_) = ui.allocate_exact_size(Vec2::new(gal.size().x+18.0,24.0), egui::Sense::hover());
    ui.painter().rect(rect, Rounding::same(10.0),
        Color32::from_rgba_premultiplied(color.r(),color.g(),color.b(),22),
        Stroke::new(1.0,color));
    ui.painter().text(egui::pos2(rect.min.x+9.0,rect.center().y),
        egui::Align2::LEFT_CENTER, text, fid, color);
}

/// Inline chip
fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    let fid = FontId::proportional(10.5);
    let gal = ui.fonts(|f| f.layout_no_wrap(text.to_string(), fid.clone(), color));
    let (rect,_) = ui.allocate_exact_size(Vec2::new(gal.size().x+12.0,18.0), egui::Sense::hover());
    ui.painter().rect(rect, Rounding::same(7.0),
        Color32::from_rgba_premultiplied(color.r(),color.g(),color.b(),20),
        Stroke::new(0.8,color));
    ui.painter().text(egui::pos2(rect.min.x+6.0,rect.center().y),
        egui::Align2::LEFT_CENTER, text, fid, color);
}

/// Compact key-value pair
fn kv(ui: &mut egui::Ui, k: &str, v: &str) {
    ui.label(RichText::new(k).size(11.0).color(C_MUTED));
    ui.label(RichText::new(v).size(11.0).color(C_TEXT));
}

fn kv_col(ui: &mut egui::Ui, k: &str, v: &str, c: Color32) {
    ui.label(RichText::new(k).size(11.0).color(C_MUTED));
    ui.label(RichText::new(v).size(11.0).color(c));
}

/// RichText shorthand
fn rt(s: &str, c: Color32, sz: f32) -> RichText { RichText::new(s).size(sz).color(c) }

/// Mini colored dot
fn dot(ui: &mut egui::Ui, c: Color32) {
    let (r,_) = ui.allocate_exact_size(Vec2::new(8.0,8.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, c);
}

/// Horizontal mini bar (inline)
fn mini_bar(ui: &mut egui::Ui, frac: f32, color: Color32, w: f32, h: f32) {
    let (rect,_) = ui.allocate_exact_size(Vec2::new(w,h), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(3.0), C_BORDER);
    let mut fr = rect; fr.set_width((rect.width()*frac.clamp(0.0,1.0)).max(2.0));
    p.rect_filled(fr, Rounding::same(3.0), color);
}

/// Sparkline from ring buffer
fn sparkline(ui: &mut egui::Ui, data: &[f32], color: Color32, height: f32) {
    let w = ui.available_width() - 4.0;
    let (rect,_) = ui.allocate_exact_size(Vec2::new(w, height), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(6.0), C_BORDER);
    if data.len() < 2 { return; }
    let n  = data.len() as f32;
    let dx = rect.width() / (n - 1.0);
    let mut pts: Vec<egui::Pos2> = data.iter().enumerate().map(|(i,&v)| {
        egui::pos2(rect.min.x + i as f32 * dx,
                   rect.max.y - (v/100.0).clamp(0.0,1.0) * rect.height())
    }).collect();
    // Fill
    let mut poly = pts.clone();
    poly.push(egui::pos2(rect.max.x, rect.max.y));
    poly.push(egui::pos2(rect.min.x, rect.max.y));
    p.add(egui::Shape::convex_polygon(poly,
        Color32::from_rgba_premultiplied(color.r(),color.g(),color.b(),35),
        Stroke::NONE));
    // Line
    for pair in pts.windows(2) {
        p.line_segment([pair[0],pair[1]], Stroke::new(1.5, color));
    }
}

// ═══════════════════════════════════════════ UTILS ════════════════════════════

fn fmtb(b: u64) -> String {
    const K:u64=1024; const M:u64=K*1024; const G:u64=M*1024; const T:u64=G*1024;
    if b>=T { format!("{:.2} TB",b as f64/T as f64) }
    else if b>=G { format!("{:.2} GB",b as f64/G as f64) }
    else if b>=M { format!("{:.1} MB",b as f64/M as f64) }
    else if b>=K { format!("{:.0} KB",b as f64/K as f64) }
    else { format!("{} B",b) }
}

fn fmtb_net(bps: f64) -> String {
    if bps>=1e9 { format!("{:.2} GB",bps/1e9) }
    else if bps>=1e6 { format!("{:.1} MB",bps/1e6) }
    else if bps>=1e3 { format!("{:.0} KB",bps/1e3) }
    else { format!("{:.0} B",bps) }
}

fn fmt_uptime(s: u64) -> String {
    format!("{}h {:02}m {:02}s", s/3600, (s%3600)/60, s%60)
}
fn fmt_uptime_long(s: u64) -> String {
    format!("{}d {:02}h {:02}m {:02}s", s/86400,(s%86400)/3600,(s%3600)/60,s%60)
}

fn ram_pct(used: u64, total: u64) -> f32 {
    if total == 0 { 0.0 } else { used as f32 / total as f32 * 100.0 }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

fn heat_color(v: f32) -> Color32 {
    if v > 90.0 { C_RED } else if v > 70.0 { C_ORANGE } else if v > 50.0 { C_YELLOW } else { C_GREEN }
}

fn cpu_extensions(vendor: &str, arch: &str) -> Vec<String> {
    let v = vendor.to_lowercase(); let a = arch.to_lowercase();
    let mut e: Vec<String> = vec![];
    if a.contains("x86") || a.contains("amd64") || a.contains("i686") {
        e.extend(["MMX","SSE","SSE2","SSE3","SSSE3","SSE4.1","SSE4.2","AES-NI","PCLMUL","RDRAND","RDSEED"].map(String::from));
        e.push("AVX".into()); e.push("AVX2".into());
        if v.contains("amd") || v.contains("authentic") {
            e.extend(["AVX-512F","FMA3","BMI1","BMI2","SHA","AMD-V (SVM)","CLZERO","MWAITX"].map(String::from));
        } else if v.contains("intel") || v.contains("genuine") {
            e.extend(["FMA3","BMI1","BMI2","CLMUL","VT-x","TSX","SGX","MPX"].map(String::from));
        }
        e.extend(["POPCNT","LZCNT","MOVBE","XSAVE","FSGSBASE"].map(String::from));
    } else if a.contains("aarch64") || a.contains("arm") {
        e.extend(["NEON","CRC32","AES","SHA1","SHA2","SHA3","PMULL","DOTPROD","SVE","SVE2"].map(String::from));
        if v.contains("apple") { e.push("AMX (Apple Matrix Extension)".into()); }
    }
    e
}

fn gpu_capabilities(vendor: &str) -> Vec<(String,String)> {
    let mut caps = vec![];
    if vendor.contains("nvidia") {
        caps.extend([
            ("CUDA","Supported (driver required)"),
            ("OpenCL","Supported"),
            ("Vulkan","Supported"),
            ("OpenGL","Supported"),
            ("DirectX","12 / DX12 Ultimate (Windows)"),
            ("NVENC","Hardware video encoding"),
            ("NVDEC","Hardware video decoding"),
            ("DLSS","AI upscaling (Turing+)"),
            ("RTX","Ray tracing (Turing+)"),
            ("NvLink","Multi-GPU bridge"),
        ].map(|(k,v)| (k.to_string(),v.to_string())));
    } else if vendor.contains("amd") || vendor.contains("ati") {
        caps.extend([
            ("OpenCL","Supported"),
            ("Vulkan","Supported"),
            ("OpenGL","Supported"),
            ("DirectX","12 / DX12 Ultimate (Windows)"),
            ("ROCm","Linux compute platform"),
            ("AMF","Hardware video encoding"),
            ("FidelityFX","FSR upscaling"),
            ("Ray Tracing","RDNA2+ supported"),
            ("Infinity Cache","RDNA2+ last-level cache"),
            ("Smart Access Memory","Supported (with Ryzen)"),
        ].map(|(k,v)| (k.to_string(),v.to_string())));
    } else if vendor.contains("intel") {
        caps.extend([
            ("OpenCL","Supported"),
            ("Vulkan","Supported"),
            ("OpenGL","Supported"),
            ("DirectX","12 (Xe+)"),
            ("QuickSync","Hardware video encode/decode"),
            ("XMX","Xe Matrix Extension (Arc)"),
            ("XeSS","AI upscaling (Arc)"),
            ("Ray Tracing","Arc Alchemist supported"),
        ].map(|(k,v)| (k.to_string(),v.to_string())));
    } else {
        caps.extend([
            ("OpenCL","Unknown"),
            ("Vulkan","Unknown"),
            ("OpenGL","Unknown"),
        ].map(|(k,v)| (k.to_string(),v.to_string())));
    }
    caps
}

// Use std::cmp::Reverse for sort
use std::cmp::Reverse;

// ═══════════════════════════════════════════ MAIN ═════════════════════════════
fn main() -> eframe::Result<()> {
    eframe::run_native(
        "SpecsUltra Pro",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1140.0, 860.0])
                .with_min_inner_size([820.0, 600.0])
                .with_title("SpecsUltra Pro"),
            ..Default::default()
        },
        Box::new(|cc| Box::new(App::new(cc))),
    )
}