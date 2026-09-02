//! Les produits couverts, et leur multiplicateur.
//!
//! C'est le piège maison du projet, et il a déjà mordu : se tromper de contrat ne
//! produit aucune erreur visible. Un GEX cinq fois trop grand reste un nombre
//! plausible, un put wall reste à un strike crédible, et rien dans la sortie ne
//! signale quoi que ce soit. Le moteur Python avait ce défaut — il lisait le
//! multiplicateur sur le drapeau de source, si bien qu'un relevé lu depuis le
//! disque retombait sur les x100 des actions.
//!
//! D'où la règle ici : on ne devine jamais. Un produit inconnu est refusé.

/// Multiplicateur d'un contrat, en unités de sous-jacent.
///
/// NQ est le **E-mini**, à x20. « E-mini » est un nom d'époque et non une mise en
/// garde : NQ était le petit frère du ND à x100 quand il a été créé, et le ND a
/// depuis disparu. À ne pas confondre avec MNQ, le Micro, qui vaut x2.
const MULTIPLICATEURS: [(&str, f64); 8] = [
    ("NQ", 20.0),
    ("ES", 50.0),
    ("CL", 1_000.0),
    ("GC", 100.0),
    ("6E", 125_000.0),
    ("6B", 62_500.0),
    ("6J", 12_500_000.0),
    ("6A", 100_000.0),
];

/// Le produit demandé n'est pas au catalogue.
///
/// Type dédié plutôt que `Option<f64>` : l'appelant doit pouvoir dire lesquels
/// existent, sans quoi l'utilisateur ne saurait pas quoi corriger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProduitInconnu {
    /// Le code demandé, normalisé en majuscules — pour le citer dans l'erreur.
    pub demande: String,
}

impl std::fmt::Display for ProduitInconnu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Multiplicateur inconnu pour {:?}. Produits connus : {}. \
             Passe-le explicitement avec --contract-size.",
            self.demande,
            produits_connus().join(", ")
        )
    }
}

impl std::error::Error for ProduitInconnu {}

/// Les codes produits reconnus, triés — pour les messages d'erreur.
pub fn produits_connus() -> Vec<&'static str> {
    let mut v: Vec<&str> = MULTIPLICATEURS.iter().map(|(code, _)| *code).collect();
    v.sort_unstable();
    v
}

/// Le multiplicateur du produit, ou une erreur qui dit lesquels sont connus.
///
/// Aucune valeur par défaut : un défaut silencieux ferait aboutir le calcul sur
/// un chiffre faux d'un facteur entier.
pub fn multiplicateur(produit: &str) -> Result<f64, ProduitInconnu> {
    let code = produit.trim().to_ascii_uppercase();
    MULTIPLICATEURS
        .iter()
        .find(|(connu, _)| *connu == code)
        .map(|(_, taille)| *taille)
        .ok_or(ProduitInconnu { demande: code })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nq_est_le_e_mini_a_vingt() {
        assert_eq!(multiplicateur("NQ").unwrap(), 20.0);
        assert_eq!(multiplicateur("nq").unwrap(), 20.0);
        assert_eq!(multiplicateur("  NQ  ").unwrap(), 20.0);
    }

    #[test]
    fn les_autres_produits_du_catalogue() {
        assert_eq!(multiplicateur("ES").unwrap(), 50.0);
        assert_eq!(multiplicateur("6E").unwrap(), 125_000.0);
    }

    /// La régression qui compte : x100 est le multiplicateur des actions, et le
    /// moteur Python y retombait sur un relevé lu depuis le disque.
    #[test]
    fn aucun_produit_ne_vaut_cent_par_defaut() {
        assert!(multiplicateur("TSLA").is_err());
        assert!(multiplicateur("SPX").is_err());
        assert!(multiplicateur("").is_err());
    }

    #[test]
    fn le_refus_nomme_les_produits_connus() {
        let err = multiplicateur("ZZZZ").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("ZZZZ"), "l'erreur doit citer la demande");
        assert!(
            message.contains("NQ"),
            "l'erreur doit lister les produits connus"
        );
        assert!(message.contains("--contract-size"), "et dire quoi faire");
    }
}
