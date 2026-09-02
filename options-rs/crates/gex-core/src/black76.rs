//! Black-76 : les options dont le sous-jacent est un future.
//!
//! Ce n'est pas Black-Scholes en habit neuf. Le sous-jacent ne porte ni dividende
//! ni coût de portage, et la prime entière s'actualise au taux sans risque. Les
//! fondre en une seule fonction à deux régimes ferait exactement l'erreur qu'on
//! ne voit pas passer : un gamma calculé dans le mauvais régime reste un nombre
//! plausible.

use crate::loi_normale::{densite, repartition};

/// Sens de l'option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sens {
    /// Droit d'acheter le future au strike.
    Call,
    /// Droit de le vendre au strike.
    Put,
}

impl Sens {
    /// Valeur intrinsèque : ce qui reste quand il n'y a plus ni temps ni vol.
    #[inline]
    fn intrinseque(self, f: f64, k: f64) -> f64 {
        match self {
            Sens::Call => (f - k).max(0.0),
            Sens::Put => (k - f).max(0.0),
        }
    }
}

#[inline]
fn d1(f: f64, k: f64, vol: f64, t: f64) -> f64 {
    ((f / k).ln() + 0.5 * vol * vol * t) / (vol * t.sqrt())
}

/// Les entrées pour lesquelles la formule a un sens.
///
/// Hors de ce domaine on ne renvoie pas une erreur mais la valeur limite : une
/// chaîne réelle porte toujours des lignes sans IV ou déjà échues, et les écarter
/// une à une chez chaque appelant encombrerait tout le moteur.
#[inline]
fn domaine_valide(f: f64, k: f64, vol: f64, t: f64) -> bool {
    t > 0.0 && vol > 0.0 && k > 0.0 && f > 0.0
}

/// Prime d'une option sur future.
pub fn prix(f: f64, k: f64, vol: f64, t: f64, r: f64, sens: Sens) -> f64 {
    if !domaine_valide(f, k, vol, t) {
        return sens.intrinseque(f, k);
    }
    let d1 = d1(f, k, vol, t);
    let d2 = d1 - vol * t.sqrt();
    let actualisation = (-r * t).exp();
    match sens {
        Sens::Call => actualisation * (f * repartition(d1) - k * repartition(d2)),
        Sens::Put => actualisation * (k * repartition(-d2) - f * repartition(-d1)),
    }
}

/// Gamma d'une option sur future. À r = 0, identique au gamma Black-Scholes.
///
/// Hors domaine il vaut zéro, et non NaN : un strike échu ou sans IV ne contribue
/// pas à l'exposition, alors qu'un NaN contaminerait la somme entière.
pub fn gamma(f: f64, k: f64, vol: f64, t: f64, r: f64) -> f64 {
    if !domaine_valide(f, k, vol, t) {
        return 0.0;
    }
    (-r * t).exp() * densite(d1(f, k, vol, t)) / (f * vol * t.sqrt())
}

/// Bornes de recherche de la volatilité implicite, et tolérance.
///
/// Les mêmes que celles du dépôt Python. Deux moteurs qui inverseraient la vol
/// dans des bornes différentes rendraient des IV différentes sur les primes
/// limites, et comparer leurs GEX ne prouverait plus rien.
const VOL_MIN: f64 = 1e-6;
const VOL_MAX: f64 = 5.0;
const TOLERANCE: f64 = 1e-8;
const ITERATIONS_MAX: u32 = 200;

/// Inverse la volatilité depuis une prime observée.
///
/// `None` plutôt qu'une valeur de repli quand la prime passe sous l'intrinsèque
/// actualisé : aucune volatilité ne la produit, et en inventer une ferait entrer
/// un chiffre faux dans le calcul sans que rien ne le signale.
pub fn volatilite_implicite(prime: f64, f: f64, k: f64, t: f64, r: f64, sens: Sens) -> Option<f64> {
    if !prime.is_finite() || t <= 0.0 {
        return None;
    }
    let plancher = sens.intrinseque(f, k) * (-r * t).exp();
    if prime <= plancher {
        return None;
    }
    brent(
        |vol| prix(f, k, vol, t, r, sens) - prime,
        VOL_MIN,
        VOL_MAX,
        TOLERANCE,
        ITERATIONS_MAX,
    )
}

/// Méthode de Brent : la dichotomie garantit la convergence, la sécante et
/// l'interpolation quadratique inverse la rendent rapide.
///
/// Écrite ici plutôt qu'empruntée à une crate : c'est soixante lignes, et une
/// dépendance aurait coûté plus cher à auditer qu'à écrire. La structure suit
/// `zbrent` à la lettre — s'en écarter « pour simplifier » est le moyen le plus
/// sûr de perdre la garantie de convergence sans que les tests le voient.
fn brent<F>(f: F, x1: f64, x2: f64, tol: f64, iterations: u32) -> Option<f64>
where
    F: Fn(f64) -> f64,
{
    let (mut a, mut b) = (x1, x2);
    let (mut fa, mut fb) = (f(a), f(b));
    // Sans changement de signe aux bornes, rien ne garantit une racine entre
    // elles : chercher quand même rendrait une valeur qui n'en est pas une.
    if !fa.is_finite() || !fb.is_finite() || (fa > 0.0) == (fb > 0.0) {
        return None;
    }
    let (mut c, mut fc) = (b, fb);
    let (mut d, mut e) = (0.0_f64, 0.0_f64);

    for _ in 0..iterations {
        if (fb > 0.0) == (fc > 0.0) {
            // b et c ont cessé d'encadrer : on réencadre avec a.
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            // On garde en b le meilleur candidat, en c celui qui encadre.
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 0.5 * tol;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb == 0.0 {
            return Some(b);
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            let s = fb / fa;
            let (mut p, mut q) = if a == c {
                (2.0 * xm * s, 1.0 - s)
            } else {
                let q = fa / fc;
                let r = fb / fc;
                (
                    s * (2.0 * xm * q * (q - r) - (b - a) * (r - 1.0)),
                    (q - 1.0) * (r - 1.0) * (s - 1.0),
                )
            };
            if p > 0.0 {
                q = -q;
            }
            p = p.abs();
            // L'interpolation n'est retenue que si elle reste dans l'encadrement
            // et progresse assez ; sinon la dichotomie, plus lente mais sûre.
            let min1 = 3.0 * xm * q - (tol1 * q).abs();
            let min2 = (e * q).abs();
            if 2.0 * p < min1.min(min2) {
                e = d;
                d = p / q;
            } else {
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        a = b;
        fa = fb;
        b += if d.abs() > tol1 {
            d
        } else if xm >= 0.0 {
            tol1
        } else {
            -tol1
        };
        fb = f(b);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Le relevé IB réel du 25 août 2026 : NQ septembre à 29 267,25.
    const F: f64 = 29_267.25;
    const VOL: f64 = 0.18;
    const T: f64 = 0.019;

    /// Valeurs figées depuis le moteur Python sur ce relevé (voir `fixtures/`).
    /// C'est l'oracle commun : sans lui, les deux moteurs pourraient se tromper
    /// identiquement sans que rien ne le révèle.
    /// Colonnes : strike, gamma, prime du call, prime du put.
    const ORACLE: [(f64, f64, f64, f64); 5] = [
        (
            28_000.00,
            1.094_178_164_645_857e-4,
            1277.8029889087,
            10.5529889087,
        ),
        (
            29_000.00,
            5.107_053_863_433_261e-4,
            441.4765816056,
            174.2265816056,
        ),
        (
            29_267.25,
            5.493_453_136_212_468e-4,
            289.6877623753,
            289.6877623753,
        ),
        (
            29_500.00,
            5.241_228_024_444_843e-4,
            189.1608810281,
            421.9108810281,
        ),
        (
            30_500.00,
            1.407_260_207_652_456e-4,
            14.8320621669,
            1247.5820621669,
        ),
    ];

    #[test]
    fn gamma_egale_le_moteur_python() {
        for (k, attendu, _, _) in ORACLE {
            let obtenu = gamma(F, k, VOL, T, 0.0);
            assert!(
                (obtenu - attendu).abs() <= 1e-12 * attendu,
                "gamma(K={k}) = {obtenu:e}, Python donne {attendu:e}"
            );
        }
    }

    #[test]
    fn prix_egale_le_moteur_python() {
        for (k, _, call, put) in ORACLE {
            for (sens, attendu) in [(Sens::Call, call), (Sens::Put, put)] {
                let obtenu = prix(F, k, VOL, T, 0.0, sens);
                assert!(
                    (obtenu - attendu).abs() <= 1e-9 * attendu.abs(),
                    "prix({sens:?}, K={k}) = {obtenu}, Python donne {attendu}"
                );
            }
        }
    }

    /// À taux nul, C - P = F - K. C'est cette identité qui permet de déduire le
    /// prix du future d'une chaîne qui ne le porterait pas.
    #[test]
    fn parite_call_put_tient() {
        for k in [28_000.0, 29_267.25, 30_500.0] {
            let c = prix(F, k, VOL, T, 0.0, Sens::Call);
            let p = prix(F, k, VOL, T, 0.0, Sens::Put);
            assert!((c - p - (F - k)).abs() < 1e-8, "parité rompue en K={k}");
        }
    }

    #[test]
    fn volatilite_implicite_retrouve_la_vol_injectee() {
        for k in [28_000.0, 29_000.0, 29_267.25, 30_500.0] {
            for sens in [Sens::Call, Sens::Put] {
                let prime = prix(F, k, VOL, T, 0.0, sens);
                let retrouvee = volatilite_implicite(prime, F, k, T, 0.0, sens)
                    .unwrap_or_else(|| panic!("aucune vol trouvée en K={k} {sens:?}"));
                assert!(
                    (retrouvee - VOL).abs() < 1e-6,
                    "vol retrouvée {retrouvee} au lieu de {VOL} en K={k}"
                );
            }
        }
    }

    /// Sur un aller-retour à plusieurs régimes de vol : c'est la robustesse de
    /// Brent qu'on mesure, pas un cas particulier bien choisi.
    #[test]
    fn volatilite_implicite_tient_sur_toute_la_plage() {
        for vol in [0.05_f64, 0.10, 0.35, 0.80, 1.50] {
            let prime = prix(F, 29_500.0, vol, 0.25, 0.02, Sens::Call);
            let retrouvee =
                volatilite_implicite(prime, F, 29_500.0, 0.25, 0.02, Sens::Call).unwrap();
            assert!((retrouvee - vol).abs() < 1e-6, "vol {vol} mal inversée");
        }
    }

    #[test]
    fn volatilite_implicite_refuse_sous_l_intrinseque() {
        // Un call mille points dans la monnaie ne peut pas valoir 1.
        assert!(volatilite_implicite(1.0, F, F - 1000.0, T, 0.0, Sens::Call).is_none());
        assert!(volatilite_implicite(f64::NAN, F, F, T, 0.0, Sens::Call).is_none());
        assert!(volatilite_implicite(100.0, F, F, 0.0, 0.0, Sens::Call).is_none());
    }

    /// Hors domaine, zéro et non NaN : un NaN contaminerait la somme entière.
    #[test]
    fn gamma_hors_domaine_vaut_zero() {
        assert_eq!(gamma(F, 29_000.0, VOL, 0.0, 0.0), 0.0);
        assert_eq!(gamma(F, 29_000.0, 0.0, T, 0.0), 0.0);
        assert_eq!(gamma(F, -1.0, VOL, T, 0.0), 0.0);
        assert_eq!(gamma(0.0, 29_000.0, VOL, T, 0.0), 0.0);
    }

    #[test]
    fn prix_a_l_echeance_vaut_l_intrinseque() {
        assert_eq!(prix(F, 29_000.0, VOL, 0.0, 0.0, Sens::Call), F - 29_000.0);
        assert_eq!(prix(F, 30_000.0, VOL, 0.0, 0.0, Sens::Call), 0.0);
        assert_eq!(prix(F, 30_000.0, VOL, 0.0, 0.0, Sens::Put), 30_000.0 - F);
    }
}
