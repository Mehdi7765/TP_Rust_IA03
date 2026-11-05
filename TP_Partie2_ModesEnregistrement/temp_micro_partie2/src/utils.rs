// Initialisation de la carte et du microphone : FOURNI, NE PAS MODIFIER.
//
// Le microphone numérique (IMP34DT05, sur le shield X-NUCLEO-IKS02A1) est lu
// par le périphérique DFSDM du STM32 :
//   - PB0  (DFSDM1_CKOUT)  : horloge envoyée au microphone (~2 MHz)
//   - PB10 (DFSDM1_DATIN7) : flux de bits (PDM) renvoyé par le microphone
// Les deux fils blancs relient ces broches aux broches D3/D4 du shield.
// Le filtre sinc3 (suréchantillonnage 250) transforme le flux de bits en
// valeurs 24 bits signées, à ~8 000 échantillons par seconde.

use hal::{
    delay::DelayFromCountDownTimer,
    gpio::{Edge, ExtiPin, Input, Output, PB14, PC13, PE1, PushPull},
    pac,
    prelude::*,
    rcc::ResetEnable,
    timer::Timer,
};
use stm32h7xx_hal as hal;

/// Convertit une valeur 24 bits en complément à deux en i32
fn i24_to_i32(n: u32) -> i32 {
    let n = n & 0x00FFFFFF;
    if n & 0x800000 != 0 {
        (n | 0xFF000000) as i32
    } else {
        n as i32
    }
}

/// Initialise l'horloge, les LED rouge et jaune, le bouton bleu (interruption
/// EXTI15_10 sur front montant) et le DFSDM.
/// La LED verte n'est pas disponible : PB0 sert d'horloge au microphone.
pub fn init(mut dp: pac::Peripherals) -> (
    DelayFromCountDownTimer<Timer<pac::TIM15>>,
    PB14<Output<PushPull>>,
    PE1<Output<PushPull>>,
    PC13<Input>,
    pac::DFSDM,
) {
    // Alimentation et horloges (horloge par défaut : HSI 64 MHz)
    let pwrcfg = dp.PWR.constrain().freeze();
    let ccdr = dp.RCC.constrain().freeze(pwrcfg, &dp.SYSCFG);

    let delay = DelayFromCountDownTimer::new(dp.TIM15.timer(
        1.kHz(),
        ccdr.peripheral.TIM15,
        &ccdr.clocks,
    ));

    // GPIO
    let gpiob = dp.GPIOB.split(ccdr.peripheral.GPIOB);
    let gpioc = dp.GPIOC.split(ccdr.peripheral.GPIOC);
    let gpioe = dp.GPIOE.split(ccdr.peripheral.GPIOE);

    let led_yellow = gpioe.pe1.into_push_pull_output();
    let led_red = gpiob.pb14.into_push_pull_output();
    let mut btn = gpioc.pc13.into_floating_input();
    btn.make_interrupt_source(&mut dp.SYSCFG);
    btn.trigger_on_edge(&mut dp.EXTI, Edge::Rising);
    btn.enable_interrupt(&mut dp.EXTI);
    // Fonction alternative 6 : DFSDM1_CKOUT (PB0) et DFSDM1_DATIN7 (PB10)
    gpiob.pb0.into_alternate::<6>();
    gpiob.pb10.into_alternate::<6>();

    // Activation du DFSDM
    ccdr.peripheral.DFSDM1.enable().reset();
    unsafe { while !(*pac::RCC::ptr()).cr.read().d2ckrdy().bit() {} }
    let _ = dp.DFSDM.flt0.isr.read().bits();

    // Configuration du DFSDM
    // Canal 0 : diviseur de l'horloge de sortie (CKOUT)
    dp.DFSDM
        .ch0
        .cfgr1
        .write(|w| unsafe { w.ckoutdiv().bits(7) });
    // Canal 7 : entrée série cadencée par CKOUT, échantillonnée sur front montant
    dp.DFSDM
        .ch7
        .cfgr1
        .write(|w| unsafe { w.spicksel().bits(1).sitp().bits(0b00) });
    dp.DFSDM
        .ch7
        .cfgr2
        .write(|w| unsafe { w.offset().bits(0xFDFFFF) });
    // Filtre 0 : lit le canal 7, filtre sinc3, suréchantillonnage 250
    dp.DFSDM
        .flt0
        .cr1
        .write(|w| unsafe { w.fast().set_bit().rch().bits(7) });
    dp.DFSDM
        .flt0
        .fcr
        .write(|w| unsafe { w.ford().bits(3).fosr().bits(249) });

    // Activation
    dp.DFSDM.ch7.cfgr1.modify(|_, w| w.chen().set_bit());
    dp.DFSDM.flt0.cr1.modify(|_, w| w.dfen().set_bit());
    dp.DFSDM.ch0.cfgr1.modify(|_, w| w.dfsdmen().set_bit());

    // Démarrage des conversions en continu
    dp.DFSDM
        .flt0
        .cr1
        .modify(|_, w| w.rcont().set_bit().rswstart().set_bit());

    (delay, led_red, led_yellow, btn, dp.DFSDM)
}

/// Attend le prochain échantillon du microphone et le renvoie (24 bits signés)
pub fn get_value(dfsdm: &pac::DFSDM) -> i32 {
    while !dfsdm.flt0.isr.read().reocf().bit() {}
    let v = dfsdm.flt0.rdatar.read().rdata().bits();
    i24_to_i32(v)
}
