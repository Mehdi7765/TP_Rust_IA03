// ============================================================================
// PARTIE 1 : DÉCOUVERTE DU MICROPHONE (rien à coder)
//
// Ce programme affiche le niveau sonore toutes les 0,1 s :
//
//     |#######...................| 15068
//     |###################.......| 512000 jaune
//
//  - la barre : échelle logarithmique (un '#' par demi-octave), comme un vumètre
//  - le nombre : amplitude maximale pendant ces 0,1 s, sans le décalage constant
//  - "jaune" / "ROUGE" : le seuil de la LED correspondante a été dépassé
//
// Les LED s'allument à chaque échantillon qui dépasse leur seuil :
//   jaune > 2^18 (262 144), rouge > 2^20 (1 048 576)
//
// Lancer avec : cargo run --release
// (Mettre RAW_OUTPUT à true pour voir chaque échantillon brut : illisible,
//  et l'affichage est trop lent pour suivre le microphone.)
// ============================================================================

#![no_main]
#![no_std]

use cortex_m::peripheral::DWT;
use stm32h7xx_hal::pac;

use defmt::println;
use defmt_rtt as _;
use panic_probe as _;

mod utils;

/// true : affiche chaque échantillon brut en 3 octets (poids faible, moyen, fort).
/// false : une ligne de résumé toutes les REPORT_PERIOD_MS.
const RAW_OUTPUT: bool = false;

const REPORT_PERIOD_MS: u32 = 100;
const YELLOW_THRESHOLD: i32 = 1 << 18;
const RED_THRESHOLD: i32 = 1 << 20;
const BAR_WIDTH: usize = 26;

/// Fréquence du processeur, relue dans les registres RCC (utils::init garde l'horloge HSI par défaut)
fn cpu_clock_hz() -> Option<u32> {
    let rcc = unsafe { &*pac::RCC::ptr() };
    if rcc.cfgr.read().sws().bits() != 0 {
        return None; // le processeur ne tourne pas sur HSI
    }
    let hsi = 64_000_000 >> rcc.cr.read().hsidiv().bits();
    let divider = match rcc.d1cfgr.read().d1cpre().bits() {
        0..=7 => 1,
        8 => 2,
        9 => 4,
        10 => 8,
        11 => 16,
        12 => 64,
        13 => 128,
        14 => 256,
        _ => 512,
    };
    Some(hsi / divider)
}

/// Barre logarithmique : un '#' par demi-octave au-dessus de 2^10,
/// pour que la voix et un claquement de mains restent lisibles
fn bar(level: u32) -> [u8; BAR_WIDTH] {
    let steps = if level < 1 << 10 {
        0
    } else {
        let octave = 31 - level.leading_zeros();
        let half = (level >> (octave - 1)) & 1;
        ((octave - 10) * 2 + half) as usize
    };
    let mut s = [b'.'; BAR_WIDTH];
    for c in s.iter_mut().take(steps.min(BAR_WIDTH)) {
        *c = b'#';
    }
    s
}

#[cortex_m_rt::entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().expect("Cannot take peripherals");
    let mut cp = cortex_m::Peripherals::take().expect("Cannot take core peripherals");
    // Compteur de cycles du processeur, pour mesurer le temps
    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();

    let (mut _delay, mut led_red, mut led_yellow, _btn, dfsdm) = utils::init(dp);

    let cpu_hz = cpu_clock_hz();
    match cpu_hz {
        Some(hz) => println!("Microphone pret (processeur a {} MHz)", hz / 1_000_000),
        None => println!("Microphone pret (frequence processeur inconnue : durees fausses)"),
    }
    let period = cpu_hz.unwrap_or(64_000_000) / 1000 * REPORT_PERIOD_MS;

    // Le microphone a un décalage constant : on le suit avec une moyenne glissante lente (x1024)
    let mut offset_x1024: i64 = 0;
    // Statistiques de la fenêtre en cours
    let mut window_start = DWT::cycle_count();
    let mut samples: u32 = 0;
    let mut level: u32 = 0;
    let mut max_abs: i32 = 0;
    let mut reports: u32 = 0;

    loop {
        let v = utils::get_value(&dfsdm);
        if RAW_OUTPUT {
            println!("{=u8:02x} {=u8:02x} {=u8:02x}", (v & 0x0000FF) as u8, ((v & 0x00FF00) >> 8) as u8, ((v & 0xFF0000) >> 16) as u8);
        }

        // LED : à chaque échantillon
        let abs_v = v.abs();
        if abs_v > YELLOW_THRESHOLD {
            led_yellow.set_high();
        } else {
            led_yellow.set_low();
        }
        if abs_v > RED_THRESHOLD {
            led_red.set_high();
        } else {
            led_red.set_low();
        }

        // Statistiques
        let offset = (offset_x1024 >> 10) as i32;
        offset_x1024 += (v - offset) as i64;
        level = level.max((v - offset).unsigned_abs());
        max_abs = max_abs.max(abs_v);
        samples += 1;

        if !RAW_OUTPUT && DWT::cycle_count().wrapping_sub(window_start) >= period {
            reports += 1;
            // La première seconde est ignorée : le filtre et le décalage se stabilisent
            if reports == 10 {
                println!("Frequence d'echantillonnage : {} echantillons/s", samples * (1000 / REPORT_PERIOD_MS));
            } else if reports > 10 {
                let label = if max_abs > RED_THRESHOLD {
                    "ROUGE"
                } else if max_abs > YELLOW_THRESHOLD {
                    "jaune"
                } else {
                    ""
                };
                let b = bar(level);
                let b = core::str::from_utf8(&b).unwrap_or("");
                println!("|{=str}| {} {=str}", b, level, label);
            }
            window_start = DWT::cycle_count();
            samples = 0;
            level = 0;
            max_abs = 0;
        }
    }
}
