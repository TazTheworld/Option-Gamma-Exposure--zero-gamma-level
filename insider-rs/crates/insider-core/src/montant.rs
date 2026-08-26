//! Ce qu'une déclaration dit d'un montant, et ce qu'elle n'en dit pas.
//!
//! Les élus déclarent des **fourchettes**, jamais des valeurs exactes :
//! « $1,001 - $15,000 » est ce que la loi impose et ce que le formulaire
//! contient. Aucun traitement ne retrouvera le chiffre réel.
//!
//! D'où un type dédié plutôt qu'un `f64`. Un flottant inviterait à sommer, et
//! sommer des fourchettes n'a de sens que si l'on dit laquelle de leurs bornes
//! on additionne — sommer des planchers sous-estime autant que sommer des
//! plafonds surestime.

/// Un montant tel qu'il est déclaré.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Montant {
    /// Une fourchette bornée des deux côtés.
    Fourchette {
        /// La borne basse déclarée.
        bas: f64,
        /// La borne haute déclarée.
        haut: f64,
    },
    /// « Over $50,000,000 » : un plancher, et rien au-dessus.
    ///
    /// Variante distincte plutôt qu'un `haut` optionnel : le plafond n'est pas
    /// une donnée manquante qu'on pourrait combler, il n'existe pas. En inventer
    /// un fausserait toute somme, et le type le rend impossible à oublier.
    AuMoins {
        /// Le plancher déclaré. Il n'y a rien au-dessus.
        bas: f64,
    },
    /// Le champ était vide ou illisible.
    Absent,
}

impl Montant {
    /// La borne basse, s'il y en a une.
    pub fn bas(&self) -> Option<f64> {
        match self {
            Montant::Fourchette { bas, .. } | Montant::AuMoins { bas } => Some(*bas),
            Montant::Absent => None,
        }
    }

    /// La borne haute. `None` sur un plafond ouvert **et** sur un montant absent
    /// — deux raisons différentes de ne pas savoir, mais aucune n'autorise à
    /// inventer un nombre.
    pub fn haut(&self) -> Option<f64> {
        match self {
            Montant::Fourchette { haut, .. } => Some(*haut),
            _ => None,
        }
    }

    /// Le milieu de la fourchette.
    ///
    /// Une commodité, pas une mesure : il n'existe que si les deux bornes
    /// existent. Sur un plafond ouvert il n'en est même pas une.
    pub fn milieu(&self) -> Option<f64> {
        match self {
            Montant::Fourchette { bas, haut } => Some((bas + haut) / 2.0),
            _ => None,
        }
    }

    /// Le montant est-il exploitable pour un calcul ?
    pub fn est_borne(&self) -> bool {
        matches!(self, Montant::Fourchette { .. })
    }
}

/// Un nombre écrit à l'américaine, virgules comprises.
fn nombre(brut: &str) -> Option<f64> {
    brut.replace(',', "").trim().parse::<f64>().ok()
}

/// Lit un montant déclaré.
///
/// Écrit à la main plutôt qu'avec une expression rationnelle : le format tient en
/// trois cas, et une dépendance de plus pour les couvrir coûterait davantage à
/// auditer qu'à écrire. Les deux chambres servent la même syntaxe.
pub fn lire(brut: &str) -> Montant {
    let texte = brut.trim();
    if texte.is_empty() {
        return Montant::Absent;
    }

    // « Over $50,000,000 » — testé AVANT la fourchette, sinon le tiret d'un
    // « $50,000,000 - » imaginaire l'emporterait.
    let minuscules = texte.to_ascii_lowercase();
    if let Some(reste) = minuscules.strip_prefix("over")
        && let Some(bas) = nombre(&reste.replace('$', ""))
    {
        return Montant::AuMoins { bas };
    }

    // « $1,001 - $15,000 ». Le séparateur est un tiret entouré d'espaces ; un
    // tiret collé appartiendrait au nombre.
    if let Some((gauche, droite)) = texte.split_once('-')
        && let (Some(bas), Some(haut)) = (
            nombre(&gauche.replace('$', "")),
            nombre(&droite.replace('$', "")),
        )
    {
        return Montant::Fourchette { bas, haut };
    }

    Montant::Absent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_fourchette_se_lit_des_deux_cotes() {
        assert_eq!(
            lire("$1,001 - $15,000"),
            Montant::Fourchette {
                bas: 1_001.0,
                haut: 15_000.0
            }
        );
        assert_eq!(
            lire("$15,001 - $50,000"),
            Montant::Fourchette {
                bas: 15_001.0,
                haut: 50_000.0
            }
        );
    }

    /// « Over $50,000,000 » n'a pas de plafond. En fabriquer un fausserait toute
    /// somme, et le type le rend impossible à oublier.
    #[test]
    fn un_plafond_ouvert_n_est_pas_une_fourchette() {
        let m = lire("Over $50,000,000");
        assert_eq!(m, Montant::AuMoins { bas: 50_000_000.0 });
        assert_eq!(m.bas(), Some(50_000_000.0));
        assert_eq!(m.haut(), None);
        assert_eq!(m.milieu(), None);
        assert!(!m.est_borne());
    }

    #[test]
    fn le_milieu_n_existe_que_borne_des_deux_cotes() {
        assert_eq!(lire("$1,001 - $15,000").milieu(), Some(8_000.5));
        assert_eq!(lire("").milieu(), None);
    }

    #[test]
    fn un_champ_vide_ou_illisible_est_absent() {
        for brut in ["", "   ", "--", "non communiqué", "$"] {
            assert_eq!(lire(brut), Montant::Absent, "{brut:?}");
        }
    }

    /// Les deux chambres servent la même syntaxe, aux espaces près.
    #[test]
    fn les_espaces_ne_changent_rien() {
        let attendu = Montant::Fourchette {
            bas: 1_001.0,
            haut: 15_000.0,
        };
        for brut in [
            "$1,001 - $15,000",
            "  $1,001 - $15,000  ",
            "$1,001-$15,000",
            "$1,001 -$15,000",
        ] {
            assert_eq!(lire(brut), attendu, "{brut:?}");
        }
    }

    #[test]
    fn la_casse_de_over_ne_compte_pas() {
        for brut in ["Over $50,000,000", "over $50,000,000", "OVER $50,000,000"] {
            assert_eq!(lire(brut), Montant::AuMoins { bas: 50_000_000.0 }, "{brut:?}");
        }
    }
}
