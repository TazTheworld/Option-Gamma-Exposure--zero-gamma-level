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
//! Le moteur Python vectorisait avec numpy pour calculer le profil sur toute une
//! grille de niveaux sans boucle. Ici les fonctions sont scalaires et l'appelant
//! boucle : en Rust la boucle ne coûte rien, et une fonction scalaire se lit et se
//! vérifie sans avoir à raisonner sur des formes de tableaux.

use crate::black76::Sens;
use crate::loi_normale::densite;

pub use crate::temps::{JOURS_BOURSE, JOURS_CALENDAIRES};

/// Un contrat évalué à un niveau de sous-jacent donné.
///
/// Regroupé en structure parce que les six premiers paramètres voyagent toujours
/// ensemble, et surtout parce qu'ils sont **tous des flottants** : en liste
/// d'arguments, intervertir `vol` et `t` compile sans un mot et rend un chiffre
/// plausible. Nommés, l'erreur devient impossible à écrire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    /// Niveau du sous-jacent auquel on évalue — le spot courant, ou un niveau
    /// hypothétique du profil.
    pub spot: f64,
    /// Prix d'exercice.
    pub strike: f64,
    /// Volatilité implicite, en décimal.
    pub vol: f64,
    /// Temps restant, en années.
    pub t: f64,
    /// Open interest, en contrats.
    pub open_interest: f64,
    /// Multiplicateur du contrat — x20 pour le NQ.
    pub taille_contrat: f64,
    /// Taux sans risque. Nul dans tout le moteur.
    pub r: f64,
    /// Rendement du sous-jacent. Nul : un future n'en porte pas.
    pub q: f64,
}

impl Position {
    /// Une position à taux et rendement nuls, le seul cas que le moteur produit.
    pub fn nouvelle(
        spot: f64,
        strike: f64,
        vol: f64,
        t: f64,
        open_interest: f64,
        taille_contrat: f64,
    ) -> Self {
        Position {
            spot,
            strike,
            vol,
            t,
            open_interest,
            taille_contrat,
            r: 0.0,
            q: 0.0,
        }
    }

    /// Le domaine où les formules ont un sens.
    ///
    /// Hors de lui l'exposition vaut zéro : un strike échu ou sans IV ne
    /// contribue pas, alors qu'un NaN contaminerait la somme entière.
    #[inline]
    fn valide(&self) -> bool {
        self.t > 0.0 && self.vol > 0.0 && self.strike > 0.0 && self.spot > 0.0
    }

    /// d1 et d2 de Black-Scholes à r = q = 0.
    #[inline]
    fn d1_d2(&self) -> (f64, f64) {
        let racine_t = self.t.sqrt();
        let d1 = ((self.spot / self.strike).ln() + 0.5 * self.vol * self.vol * self.t)
            / (self.vol * racine_t);
        (d1, d1 - self.vol * racine_t)
    }

    /// Le facteur commun aux trois expositions : OI x multiplicateur x niveau.
    #[inline]
    fn echelle(&self) -> f64 {
        self.open_interest * self.taille_contrat * self.spot
    }
}

/// Gamma exposure : dollars par mouvement de 1 % du sous-jacent.
///
/// Les calls et les puts prennent deux formules **différentes mais
/// mathématiquement égales**. Ce n'est pas une maladresse héritée : les faire
/// diverger est le seul moyen qu'un test a de détecter une faute de frappe dans
/// l'une des deux, puisque le gamma d'un call et celui d'un put au même strike
/// sont identiques.
pub fn gamma_exposition(p: &Position, sens: Sens) -> f64 {
    if !p.valide() {
        return 0.0;
    }
    let racine_t = p.t.sqrt();
    let dp = ((p.spot / p.strike).ln() + (p.r - p.q + 0.5 * p.vol * p.vol) * p.t)
        / (p.vol * racine_t);
    let gamma = match sens {
        Sens::Call => (-p.q * p.t).exp() * densite(dp) / (p.spot * p.vol * racine_t),
        Sens::Put => {
            let dm = dp - p.vol * racine_t;
            p.strike * (-p.r * p.t).exp() * densite(dm) / (p.spot * p.spot * p.vol * racine_t)
        }
    };
    p.echelle() * p.spot * 0.01 * gamma
}

/// Charm exposure : dollars de delta gagnés par jour, à prix constant.
///
/// `charm = phi(d1) * d2 / (2T)`. À q = 0 il est identique pour calls et puts —
/// le -1 du delta put ne s'écoule pas.
///
/// La dérivée est par unité de T, donc `jours_par_an` doit valoir le **même**
/// diviseur que celui ayant servi à calculer T. Les désaccorder est l'erreur
/// silencieuse du calcul : le signe reste juste, seule l'amplitude est fausse —
/// d'où [`crate::temps::Convention::jours_par_an`], qui les tient ensemble.
///
/// Un charm positif signifie que le delta du book dealer grossit avec le temps,
/// donc qu'il doit vendre pour rester neutre. C'est le flux de couverture des
/// derniers jours avant échéance, celui que le gamma seul ne montre pas.
pub fn charm_exposition(p: &Position, jours_par_an: f64) -> f64 {
    if !p.valide() {
        return 0.0;
    }
    let (d1, d2) = p.d1_d2();
    p.echelle() * (densite(d1) * d2 / (2.0 * p.t)) / jours_par_an
}

/// Vanna exposure : dollars de delta par point de volatilité implicite.
///
/// `vanna = -phi(d1) * d2 / vol`, également identique calls et puts à q = 0.
///
/// Un vanna positif signifie que le delta du book grossit quand la volatilité
/// monte : les dealers vendent alors dans les pics de vol. C'est le canal par
/// lequel un choc de volatilité se transmet au spot.
pub fn vanna_exposition(p: &Position) -> f64 {
    if !p.valide() {
        return 0.0;
    }
    let (d1, d2) = p.d1_d2();
    p.echelle() * (-densite(d1) * d2 / p.vol) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: f64 = 29_267.25;
    const VOL: f64 = 0.18;
    const T: f64 = 0.019;
    const OI: f64 = 500.0;
    const TAILLE: f64 = 20.0;

    fn pos(strike: f64) -> Position {
        Position::nouvelle(S, strike, VOL, T, OI, TAILLE)
    }

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
            let p = pos(k);
            proche(
                gamma_exposition(&p, Sens::Call),
                g_call,
                1e-13,
                &format!("gamma call K={k}"),
            );
            proche(
                gamma_exposition(&p, Sens::Put),
                g_put,
                1e-13,
                &format!("gamma put K={k}"),
            );
            proche(
                charm_exposition(&p, JOURS_BOURSE),
                charm,
                1e-13,
                &format!("charm K={k}"),
            );
            proche(vanna_exposition(&p), vanna, 1e-13, &format!("vanna K={k}"));
        }
    }

    /// Deux formules différentes pour la même quantité : leur désaccord est le
    /// seul signal qu'une faute de frappe dans l'une produirait.
    #[test]
    fn gamma_expositions_call_et_put_coincident() {
        for k in [27_000.0, 29_000.0, 29_267.25, 31_500.0] {
            let p = pos(k);
            proche(
                gamma_exposition(&p, Sens::Put),
                gamma_exposition(&p, Sens::Call),
                1e-12,
                &format!("call vs put K={k}"),
            );
        }
    }

    /// Le charm est la dérivée du delta dollar par rapport au temps. On la
    /// recoupe par différence finie plutôt que contre une valeur figée : une
    /// constante recopiée de la même formule ne prouverait rien.
    #[test]
    fn charm_recoupe_la_derivee_du_delta_par_le_temps() {
        let h = 1e-7;
        let delta_dollar = |t: f64| {
            let p = Position::nouvelle(S, 29_500.0, VOL, t, OI, TAILLE);
            let (d1, _) = p.d1_d2();
            p.echelle() * crate::loi_normale::repartition(d1)
        };
        // Le temps s'écoule : T diminue. d(delta)/d(jour) = -d(delta)/dT / jours_par_an
        let derivee = -(delta_dollar(T + h) - delta_dollar(T - h)) / (2.0 * h) / JOURS_BOURSE;
        proche(
            charm_exposition(&pos(29_500.0), JOURS_BOURSE),
            derivee,
            1e-6,
            "charm vs différence finie",
        );
    }

    /// Idem pour la vanna : dérivée du delta dollar par rapport à la volatilité,
    /// ramenée à un point de vol (d'où le /100).
    #[test]
    fn vanna_recoupe_la_derivee_du_delta_par_la_vol() {
        let h = 1e-7;
        let delta_dollar = |vol: f64| {
            let p = Position::nouvelle(S, 29_500.0, vol, T, OI, TAILLE);
            let (d1, _) = p.d1_d2();
            p.echelle() * crate::loi_normale::repartition(d1)
        };
        let derivee = (delta_dollar(VOL + h) - delta_dollar(VOL - h)) / (2.0 * h) / 100.0;
        proche(
            vanna_exposition(&pos(29_500.0)),
            derivee,
            1e-6,
            "vanna vs différence finie",
        );
    }

    /// Le gamma exposure est la dérivée seconde du prix, mise à l'échelle de 1 %.
    #[test]
    fn gamma_exposition_recoupe_la_derivee_seconde() {
        let h = 1.0;
        let prime = |s: f64| crate::black76::prix(s, 29_000.0, VOL, T, 0.0, Sens::Call);
        let d2p = (prime(S + h) - 2.0 * prime(S) + prime(S - h)) / (h * h);
        proche(
            gamma_exposition(&pos(29_000.0), Sens::Call),
            OI * TAILLE * S * S * 0.01 * d2p,
            1e-5,
            "gamma vs dérivée seconde",
        );
    }

    #[test]
    fn hors_domaine_les_expositions_valent_zero() {
        for p in [
            Position::nouvelle(S, 29_000.0, VOL, 0.0, OI, TAILLE),
            Position::nouvelle(S, 29_000.0, 0.0, T, OI, TAILLE),
            Position::nouvelle(S, -1.0, VOL, T, OI, TAILLE),
            Position::nouvelle(0.0, 29_000.0, VOL, T, OI, TAILLE),
        ] {
            assert_eq!(gamma_exposition(&p, Sens::Call), 0.0);
            assert_eq!(charm_exposition(&p, JOURS_BOURSE), 0.0);
            assert_eq!(vanna_exposition(&p), 0.0);
        }
    }

    /// Un open interest nul ne contribue pas : c'est ce qui rend les murs lisibles.
    #[test]
    fn sans_open_interest_aucune_exposition() {
        let p = Position::nouvelle(S, 29_000.0, VOL, T, 0.0, TAILLE);
        assert_eq!(gamma_exposition(&p, Sens::Call), 0.0);
    }

    /// Le désaccord de diviseur ne change pas le signe, seulement l'amplitude :
    /// c'est exactement l'erreur qui ne se voit pas à la lecture.
    #[test]
    fn le_diviseur_du_charm_change_l_amplitude_pas_le_signe() {
        let p = pos(29_500.0);
        let bourse = charm_exposition(&p, JOURS_BOURSE);
        let calendaire = charm_exposition(&p, JOURS_CALENDAIRES);
        assert!(bourse.signum() == calendaire.signum());
        proche(
            bourse / calendaire,
            JOURS_CALENDAIRES / JOURS_BOURSE,
            1e-12,
            "rapport des diviseurs",
        );
    }
}
