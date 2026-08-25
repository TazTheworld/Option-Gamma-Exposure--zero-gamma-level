//! Expositions en dollars : gamma, charm, vanna, à r = q = 0.
//!
//! Ce ne sont pas les greeks nus mais les **expositions** — le greek multiplié par
//! l'open interest, la taille du contrat et le niveau du sous-jacent. C'est ce
//! chiffre-là qui a un sens : un gamma de 0,0005 ne dit rien, quatre-vingt-dix
//! millions de dollars par mouvement de 1 % si.
//!
//! À r = q = 0 ces formules sont identiquement égales à leurs équivalents
//! Black-76 : elles s'appliquent donc telles quelles aux options sur futures,
//! seule la taille du contrat change.
//!
//! Le dépôt Python vectorisait avec numpy pour calculer le profil sur toute une
//! grille de niveaux sans boucle. Ici les fonctions sont scalaires et l'appelant
//! boucle : en Rust la boucle ne coûte rien, et une fonction scalaire se lit et se
//! vérifie sans avoir à raisonner sur des formes de tableaux.

use crate::black76::Sens;
use crate::loi_normale::densite;

/// Jours de bourse dans une année, convention Perfiliev.
///
/// Sert de diviseur au charm quand le temps restant est compté en jours ouvrés.
/// Le désaccorder du diviseur ayant servi à calculer T donnerait un charm faux
/// d'un facteur 262/365, soit 40 % — sans rien changer au signe, donc sans que
/// la lecture paraisse anormale.
pub const JOURS_BOURSE: f64 = 262.0;

/// Jours calendaires, convention CBOE.
pub const JOURS_CALENDAIRES: f64 = 365.0;

/// Domaine où les formules ont un sens. Hors de lui l'exposition vaut zéro : un
/// strike échu ou sans IV ne contribue pas, alors qu'un NaN contaminerait la somme.
#[inline]
fn domaine_valide(k: f64, vol: f64, t: f64) -> bool {
    t > 0.0 && vol > 0.0 && k > 0.0
}

/// d1 et d2 de Black-Scholes à r = q = 0.
#[inline]
fn d1_d2(s: f64, k: f64, vol: f64, t: f64) -> (f64, f64) {
    let racine_t = t.sqrt();
    let d1 = ((s / k).ln() + 0.5 * vol * vol * t) / (vol * racine_t);
    (d1, d1 - vol * racine_t)
}

/// Gamma exposure : dollars par mouvement de 1 % du sous-jacent.
///
/// Les calls et les puts prennent deux formules **différentes mais
/// mathématiquement égales**. Ce n'est pas une maladresse héritée : les faire
/// diverger est le seul moyen qu'un test a de détecter une faute de frappe dans
/// l'une des deux, puisque le gamma d'un call et celui d'un put au même strike
/// sont identiques. `gamma_expositions_call_et_put_coincident` le vérifie.
pub fn gamma_exposition(
    s: f64,
    k: f64,
    vol: f64,
    t: f64,
    r: f64,
    q: f64,
    sens: Sens,
    open_interest: f64,
    taille_contrat: f64,
) -> f64 {
    if !domaine_valide(k, vol, t) || s <= 0.0 {
        return 0.0;
    }
    let racine_t = t.sqrt();
    let dp = ((s / k).ln() + (r - q + 0.5 * vol * vol) * t) / (vol * racine_t);
    let gamma = match sens {
        Sens::Call => (-q * t).exp() * densite(dp) / (s * vol * racine_t),
        Sens::Put => {
            let dm = dp - vol * racine_t;
            k * (-r * t).exp() * densite(dm) / (s * s * vol * racine_t)
        }
    };
    open_interest * taille_contrat * s * s * 0.01 * gamma
}

/// Charm exposure : dollars de delta gagnés par jour, à prix constant.
///
/// `charm = phi(d1) * d2 / (2T)`. À q = 0 il est identique pour calls et puts —
/// le -1 du delta put ne s'écoule pas.
///
/// La dérivée est par unité de T, donc `jours_par_an` doit valoir le **même**
/// diviseur que celui ayant servi à calculer T. Les désaccorder est l'erreur
/// silencieuse du calcul : le signe reste juste, seule l'amplitude est fausse.
///
/// Un charm positif signifie que le delta du book dealer grossit avec le temps,
/// donc qu'il doit vendre pour rester neutre. C'est le flux de couverture des
/// derniers jours avant échéance, celui que le gamma seul ne montre pas.
pub fn charm_exposition(
    s: f64,
    k: f64,
    vol: f64,
    t: f64,
    open_interest: f64,
    taille_contrat: f64,
    jours_par_an: f64,
) -> f64 {
    if !domaine_valide(k, vol, t) || s <= 0.0 {
        return 0.0;
    }
    let (d1, d2) = d1_d2(s, k, vol, t);
    let charm = densite(d1) * d2 / (2.0 * t);
    open_interest * taille_contrat * s * charm / jours_par_an
}

/// Vanna exposure : dollars de delta par point de volatilité implicite.
///
/// `vanna = -phi(d1) * d2 / vol`, également identique calls et puts à q = 0.
///
/// Un vanna positif signifie que le delta du book grossit quand la volatilité
/// monte : les dealers vendent alors dans les pics de vol. C'est le canal par
/// lequel un choc de volatilité se transmet au spot.
pub fn vanna_exposition(
    s: f64,
    k: f64,
    vol: f64,
    t: f64,
    open_interest: f64,
    taille_contrat: f64,
) -> f64 {
    if !domaine_valide(k, vol, t) || s <= 0.0 {
        return 0.0;
    }
    let (d1, d2) = d1_d2(s, k, vol, t);
    let vanna = -densite(d1) * d2 / vol;
    open_interest * taille_contrat * s * vanna / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: f64 = 29_267.25;
    const VOL: f64 = 0.18;
    const T: f64 = 0.019;
    const OI: f64 = 500.0;
    const TAILLE: f64 = 20.0;

    /// Figé depuis le moteur Python (voir `fixtures/`).
    /// Colonnes : strike, gamma call, gamma put, charm, vanna.
    const ORACLE: [(f64, f64, f64, f64, f64); 5] = [
        (
            28_000.00,
            9.372_422_941_166_093e6,
            9.372_422_941_166_092e6,
            4.138_027_646_501_923e6,
            -2.288_789_069_365_175e6,
        ),
        (
            29_000.00,
            4.374_558_946_431_272e7,
            4.374_558_946_431_271e7,
            3.895_410_926_132_306e6,
            -2.154_595_065_587_402e6,
        ),
        (
            29_267.25,
            4.705_537_714_392_509e7,
            4.705_537_714_392_509e7,
            -1.454_765_476_586_997e5,
            8.046_469_491_611_19e4,
        ),
        (
            29_500.00,
            4.489_488_765_487_172e7,
            4.489_488_765_487_172e7,
            -3.710_691_390_281_653e6,
            2.052_424_637_869_119e6,
        ),
        (
            30_500.00,
            1.205_419_581_614_567e7,
            1.205_419_581_614_567e7,
            -5.032_510_542_870_929e6,
            2.783_537_498_045_72e6,
        ),
    ];

    fn proche(obtenu: f64, attendu: f64, tolerance: f64, quoi: &str) {
        let ecart = ((obtenu - attendu) / attendu).abs();
        assert!(
            ecart <= tolerance,
            "{quoi} : {obtenu:e}, Python donne {attendu:e} (écart relatif {ecart:e})"
        );
    }

    #[test]
    fn expositions_egalent_le_moteur_python() {
        for (k, g_call, g_put, charm, vanna) in ORACLE {
            proche(
                gamma_exposition(S, k, VOL, T, 0.0, 0.0, Sens::Call, OI, TAILLE),
                g_call,
                1e-13,
                &format!("gamma call K={k}"),
            );
            proche(
                gamma_exposition(S, k, VOL, T, 0.0, 0.0, Sens::Put, OI, TAILLE),
                g_put,
                1e-13,
                &format!("gamma put K={k}"),
            );
            proche(
                charm_exposition(S, k, VOL, T, OI, TAILLE, JOURS_BOURSE),
                charm,
                1e-13,
                &format!("charm K={k}"),
            );
            proche(
                vanna_exposition(S, k, VOL, T, OI, TAILLE),
                vanna,
                1e-13,
                &format!("vanna K={k}"),
            );
        }
    }

    /// Deux formules différentes pour la même quantité : leur désaccord est le
    /// seul signal qu'une faute de frappe dans l'une produirait.
    #[test]
    fn gamma_expositions_call_et_put_coincident() {
        for k in [27_000.0, 29_000.0, 29_267.25, 31_500.0] {
            let c = gamma_exposition(S, k, VOL, T, 0.0, 0.0, Sens::Call, OI, TAILLE);
            let p = gamma_exposition(S, k, VOL, T, 0.0, 0.0, Sens::Put, OI, TAILLE);
            proche(p, c, 1e-12, &format!("call vs put K={k}"));
        }
    }

    /// Le charm est la dérivée du delta dollar par rapport au temps. On la
    /// recoupe par différence finie plutôt que contre une valeur figée : une
    /// constante recopiée de la même formule ne prouverait rien.
    #[test]
    fn charm_recoupe_la_derivee_du_delta_par_le_temps() {
        let k = 29_500.0;
        let h = 1e-7;
        // delta dollar d'un call, à r = q = 0
        let delta_dollar = |t: f64| {
            let (d1, _) = d1_d2(S, k, VOL, t);
            OI * TAILLE * S * crate::loi_normale::repartition(d1)
        };
        // Le temps s'écoule : T diminue. d(delta)/d(jour) = -d(delta)/dT / jours_par_an
        let derivee = -(delta_dollar(T + h) - delta_dollar(T - h)) / (2.0 * h) / JOURS_BOURSE;
        let analytique = charm_exposition(S, k, VOL, T, OI, TAILLE, JOURS_BOURSE);
        proche(analytique, derivee, 1e-6, "charm vs différence finie");
    }

    /// Idem pour la vanna : dérivée du delta dollar par rapport à la volatilité,
    /// ramenée à un point de vol (d'où le /100).
    #[test]
    fn vanna_recoupe_la_derivee_du_delta_par_la_vol() {
        let k = 29_500.0;
        let h = 1e-7;
        let delta_dollar = |vol: f64| {
            let (d1, _) = d1_d2(S, k, vol, T);
            OI * TAILLE * S * crate::loi_normale::repartition(d1)
        };
        let derivee = (delta_dollar(VOL + h) - delta_dollar(VOL - h)) / (2.0 * h) / 100.0;
        let analytique = vanna_exposition(S, k, VOL, T, OI, TAILLE);
        proche(analytique, derivee, 1e-6, "vanna vs différence finie");
    }

    /// Le gamma exposure est la dérivée seconde du prix, mise à l'échelle de 1 %.
    #[test]
    fn gamma_exposition_recoupe_la_derivee_seconde() {
        let k = 29_000.0;
        let h = 1.0;
        let prime = |s: f64| crate::black76::prix(s, k, VOL, T, 0.0, Sens::Call);
        let d2p = (prime(S + h) - 2.0 * prime(S) + prime(S - h)) / (h * h);
        let attendu = OI * TAILLE * S * S * 0.01 * d2p;
        let obtenu = gamma_exposition(S, k, VOL, T, 0.0, 0.0, Sens::Call, OI, TAILLE);
        proche(obtenu, attendu, 1e-5, "gamma vs dérivée seconde");
    }

    #[test]
    fn hors_domaine_les_expositions_valent_zero() {
        for (k, vol, t) in [(29_000.0, VOL, 0.0), (29_000.0, 0.0, T), (-1.0, VOL, T)] {
            assert_eq!(
                gamma_exposition(S, k, vol, t, 0.0, 0.0, Sens::Call, OI, TAILLE),
                0.0
            );
            assert_eq!(charm_exposition(S, k, vol, t, OI, TAILLE, JOURS_BOURSE), 0.0);
            assert_eq!(vanna_exposition(S, k, vol, t, OI, TAILLE), 0.0);
        }
    }

    /// Un open interest nul ne contribue pas : c'est ce qui rend les murs lisibles.
    #[test]
    fn sans_open_interest_aucune_exposition() {
        assert_eq!(
            gamma_exposition(S, 29_000.0, VOL, T, 0.0, 0.0, Sens::Call, 0.0, TAILLE),
            0.0
        );
    }
}
