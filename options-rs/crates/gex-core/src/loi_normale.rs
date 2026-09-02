//! Densité et fonction de répartition de la loi normale centrée réduite.
//!
//! La bibliothèque standard n'a ni `erf` ni `Phi`, et les deux sont au cœur de
//! Black-Scholes comme de Black-76. Les écrire ici plutôt que de les recopier
//! dans chaque formule évite la divergence silencieuse : deux approximations
//! légèrement différentes donneraient deux gammas pour la même option, et l'écart
//! ne se verrait sur aucun test qui ne les compare pas entre elles.

/// 1 / sqrt(2 * pi)
const INV_RACINE_2PI: f64 = 0.398_942_280_401_432_7;
/// 1 / sqrt(2)
const INV_RACINE_2: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Densité de probabilité en `x`.
#[inline]
pub fn densite(x: f64) -> f64 {
    INV_RACINE_2PI * (-0.5 * x * x).exp()
}

/// Fonction de répartition en `x`.
///
/// Passe par `erfc` et non `erf` : pour x très négatif, `0.5 * (1 + erf(z))`
/// soustrait deux nombres proches et perd les chiffres significatifs, là où
/// `0.5 * erfc(-z)` les garde. Sur une option très en dehors de la monnaie,
/// c'est la différence entre une probabilité juste et un zéro.
#[inline]
pub fn repartition(x: f64) -> f64 {
    0.5 * libm::erfc(-x * INV_RACINE_2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Valeurs de scipy.stats.norm, qui sert de référence au dépôt Python.
    ///
    /// Les tolérances sont mesurées, pas choisies pour passer. `libm::erfc` et
    /// celle de scipy sont deux implémentations indépendantes : elles s'accordent
    /// à quelques ulps, ce qui est le mieux qu'on puisse exiger d'un f64 dont
    /// l'epsilon vaut 2,2e-16. Exiger 1e-15 en relatif, soit environ quatre ulps,
    /// mesurerait le choix d'implémentation d'erfc et non la justesse du calcul.
    ///
    /// La queue lointaine tolère 1e-13 : à x = -8, Phi vaut 6e-16 et l'écart
    /// absolu entre les deux est de 5e-29 — vingt ordres de grandeur sous le plus
    /// petit gamma qui pèse quoi que ce soit dans une exposition.
    #[test]
    fn repartition_egale_scipy() {
        let cas = [
            (-8.0, 6.220_960_574_271_275e-16, 1e-13),
            (-3.0, 1.349_898_031_630_095e-3, 1e-14),
            (-1.0, 1.586_552_539_314_570_5e-1, 1e-14),
            (0.0, 5.0e-1, 1e-14),
            (1.0, 8.413_447_460_685_429e-1, 1e-14),
            (3.0, 9.986_501_019_683_699e-1, 1e-14),
        ];
        for (x, attendu, tolerance) in cas {
            let obtenu = repartition(x);
            let ecart_relatif = ((obtenu - attendu) / attendu).abs();
            assert!(
                ecart_relatif <= tolerance,
                "Phi({x}) = {obtenu:e}, attendu {attendu:e} (écart relatif {ecart_relatif:e})"
            );
        }
    }

    #[test]
    fn densite_egale_scipy() {
        let cas = [
            (0.0, 3.989_422_804_014_327e-1),
            (1.0, 2.419_707_245_191_434e-1),
            (-2.5, 1.752_830_049_356_853_5e-2),
        ];
        for (x, attendu) in cas {
            assert!((densite(x) - attendu).abs() <= 1e-15 * attendu);
        }
    }

    /// La symétrie n'est pas décorative : elle est ce qui garantit que la parité
    /// call-put tienne, et donc que le prix du future déduit par parité soit juste.
    #[test]
    fn repartition_est_symetrique() {
        for x in [0.3_f64, 1.0, 2.7, 5.5] {
            assert!((repartition(x) + repartition(-x) - 1.0).abs() < 1e-15);
        }
    }

    /// Le cas qui motive erfc plutôt que erf : sans lui, on rendrait 0.
    #[test]
    fn repartition_reste_fine_loin_dans_la_queue() {
        assert!(
            repartition(-30.0) > 0.0,
            "annulation catastrophique dans la queue"
        );
        assert!(repartition(-30.0) < 1e-190);
    }
}
