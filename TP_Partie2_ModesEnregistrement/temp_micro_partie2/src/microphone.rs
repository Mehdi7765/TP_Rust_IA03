// Lecture du microphone : FOURNI, NE PAS MODIFIER.

use stm32h7xx_hal::pac;

use crate::utils;

/// Nombre d'échantillons gardés en mémoire AVANT le déclenchement (400 = 50 ms),
/// pour ne pas perdre le début du mot.
pub const PRE_TRIGGER: usize = 400;

/// Lecteur de microphone :
///  - estime le décalage constant (offset) du signal pour calculer l'amplitude,
///  - garde les PRE_TRIGGER derniers échantillons (historique circulaire).
pub struct Microphone {
    offset_x1024: i64,
    history: [i32; PRE_TRIGGER],
    next: usize,
}

impl Microphone {
    pub fn new() -> Self {
        Self {
            offset_x1024: 0,
            history: [0; PRE_TRIGGER],
            next: 0,
        }
    }

    /// Lit un échantillon et renvoie (valeur brute, amplitude sans le décalage)
    pub fn read(&mut self, dfsdm: &pac::DFSDM) -> (i32, u32) {
        let v = utils::get_value(dfsdm);
        // Moyenne glissante lente (x1024) : suit le décalage constant du signal
        let offset = (self.offset_x1024 >> 10) as i32;
        self.offset_x1024 += (v - offset) as i64;
        self.history[self.next] = v;
        self.next = (self.next + 1) % PRE_TRIGGER;
        (v, (v - offset).unsigned_abs())
    }

    /// Copie les PRE_TRIGGER derniers échantillons dans `out`, du plus ancien au plus récent.
    /// `out` doit avoir une longueur de PRE_TRIGGER.
    pub fn copy_history(&self, out: &mut [i32]) {
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.history[(self.next + i) % PRE_TRIGGER];
        }
    }
}
