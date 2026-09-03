//! Ce qui mérite d'interrompre quelqu'un, et ce qui n'est que du bruit.
//!
//! Le collecteur voit passer un relevé toutes les quinze secondes. Trois
//! événements y changent la lecture du terrain : le GEX qui change de signe, le
//! prix qui franchit le zero gamma, le prix qui franchit un mur. Les trois sont
//! datés — ils se poussent, ils ne se consultent pas — et c'est la raison d'être
//! de ce module.
//!
//! # Tout le problème est de ne pas crier sur du bruit
//!
//! Un GEX qui oscille autour de zéro change de signe plusieurs fois par minute
//! sans que rien ne se passe. Un prix qui longe un mur le franchit vingt fois de
//! suite. Une alerte par oscillation vaudrait moins que pas d'alerte du tout :
//! elle apprend en une séance à ne plus regarder.
//!
//! Deux garde-fous, et aucun n'est un réglage de confort :
//!
//! - **Le signe du GEX doit tenir.** Un nouveau signe n'est annoncé qu'après
//!   l'avoir vu plusieurs passages d'affilée. Un aller-retour ne déclenche rien.
//! - **Les niveaux ont une bande morte.** Tant que le prix est à moins d'une
//!   marge du niveau, aucun côté ne lui est attribué — il n'est ni au-dessus ni
//!   en dessous. Un franchissement n'existe que d'un côté franc à l'autre.
//!
//! **Ce module ne parle à personne.** Il rend ce qui a basculé ; l'envoi est
//! ailleurs, et c'est ce qui le rend testable sans réseau.

/// L'état du terrain à un instant, réduit à ce qui peut déclencher.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Etat {
    /// Prix du sous-jacent.
    pub spot: f64,
    /// GEX total.
    pub gex: f64,
    /// Zero gamma retenu, s'il existe.
    pub zero_gamma: Option<f64>,
    /// Mur call en gamma.
    pub call_wall: Option<f64>,
    /// Mur put en gamma.
    pub put_wall: Option<f64>,
}

/// Ce qui a basculé depuis le passage précédent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bascule {
    /// Le GEX a changé de signe, et le nouveau signe a tenu.
    ///
    /// C'est le changement de régime : au-dessus de zéro les teneurs amortissent
    /// les mouvements, en dessous ils les amplifient.
    RegimeGamma {
        /// Le GEX au moment où le nouveau signe s'est confirmé.
        gex: f64,
        /// Le nouveau régime amplifie-t-il ?
        negatif: bool,
    },
    /// Le prix a franchi le zero gamma, d'un côté franc à l'autre.
    ZeroGamma {
        /// Le niveau franchi.
        niveau: f64,
        /// Vers le haut ?
        vers_le_haut: bool,
    },
    /// Le prix a franchi un mur.
    Mur {
        /// « call wall » ou « put wall ».
        nom: &'static str,
        /// Le niveau franchi.
        niveau: f64,
        /// Vers le haut ?
        vers_le_haut: bool,
    },
}

impl Bascule {
    /// Ce qu'on en dit, en une ligne.
    pub fn titre(&self) -> String {
        match self {
            Bascule::RegimeGamma { negatif: true, .. } => {
                "GAMMA NÉGATIF : les couvertures amplifient désormais".to_string()
            }
            Bascule::RegimeGamma { negatif: false, .. } => {
                "GAMMA POSITIF : les couvertures amortissent désormais".to_string()
            }
            Bascule::ZeroGamma { vers_le_haut, .. } => format!(
                "ZERO GAMMA FRANCHI {}",
                if *vers_le_haut {
                    "PAR LE HAUT"
                } else {
                    "PAR LE BAS"
                }
            ),
            Bascule::Mur {
                nom, vers_le_haut, ..
            } => format!(
                "{} FRANCHI {}",
                nom.to_uppercase(),
                if *vers_le_haut {
                    "PAR LE HAUT"
                } else {
                    "PAR LE BAS"
                }
            ),
        }
    }
}

/// De quel côté d'un niveau se trouve le prix, bande morte comprise.
///
/// `None` dans la bande : ni au-dessus, ni en dessous. C'est ce qui empêche un
/// prix qui longe un niveau de le « franchir » à chaque cotation — sans elle,
/// une séance passée à côté d'un mur produirait des dizaines d'alertes qui
/// décrivent la même chose.
fn cote(spot: f64, niveau: f64, marge: f64) -> Option<bool> {
    let bande = niveau.abs() * marge;
    if spot > niveau + bande {
        Some(true)
    } else if spot < niveau - bande {
        Some(false)
    } else {
        None
    }
}

/// Nombre de passages pendant lesquels un nouveau signe doit tenir.
pub const CONFIRMATIONS_DEFAUT: u32 = 3;

/// Demi-largeur de la bande morte autour d'un niveau, en fraction de ce niveau.
///
/// Un millième : sur un NQ à 29 000, cela fait vingt-neuf points. En deçà, le
/// prix est considéré comme collé au niveau, ni d'un côté ni de l'autre.
pub const MARGE_DEFAUT: f64 = 0.001;

/// La mémoire qui distingue un basculement d'une oscillation.
#[derive(Debug, Clone)]
pub struct Veilleuse {
    signe_confirme: Option<i8>,
    candidat: Option<(i8, u32)>,
    cote_zero: Option<bool>,
    cote_call: Option<bool>,
    cote_put: Option<bool>,
    amorcee: bool,
    confirmations: u32,
    marge: f64,
}

impl Default for Veilleuse {
    fn default() -> Self {
        Veilleuse::neuve(CONFIRMATIONS_DEFAUT, MARGE_DEFAUT)
    }
}

impl Veilleuse {
    /// Une veilleuse vierge.
    pub fn neuve(confirmations: u32, marge: f64) -> Veilleuse {
        Veilleuse {
            signe_confirme: None,
            candidat: None,
            cote_zero: None,
            cote_call: None,
            cote_put: None,
            amorcee: false,
            // Zéro confirmation n'aurait pas de sens : le premier passage
            // deviendrait sa propre confirmation, et la garde disparaîtrait.
            confirmations: confirmations.max(1),
            marge: marge.max(0.0),
        }
    }

    /// Observe un état, et rend ce qui a basculé.
    ///
    /// **Le premier passage n'annonce rien.** Il enregistre où l'on se trouve :
    /// sans lui, un démarrage de collecteur en gamma négatif annoncerait un
    /// basculement qui n'a pas eu lieu, et le premier relevé du matin crierait
    /// tous les jours.
    pub fn observer(&mut self, etat: &Etat) -> Vec<Bascule> {
        let mut sorties = Vec::new();

        let signe = if etat.gex > 0.0 {
            1
        } else if etat.gex < 0.0 {
            -1
        } else {
            0
        };
        let z = etat.zero_gamma.and_then(|n| cote(etat.spot, n, self.marge));
        let c = etat.call_wall.and_then(|n| cote(etat.spot, n, self.marge));
        let p = etat.put_wall.and_then(|n| cote(etat.spot, n, self.marge));

        if !self.amorcee {
            self.amorcee = true;
            self.signe_confirme = Some(signe);
            (self.cote_zero, self.cote_call, self.cote_put) = (z, c, p);
            return sorties;
        }

        // Le signe : il doit tenir avant d'être annoncé.
        match self.signe_confirme {
            Some(confirme) if confirme == signe => self.candidat = None,
            _ => {
                let compte = match self.candidat {
                    Some((s, n)) if s == signe => n + 1,
                    _ => 1,
                };
                if compte >= self.confirmations {
                    self.signe_confirme = Some(signe);
                    self.candidat = None;
                    // Un GEX exactement nul n'est pas un régime : ne rien
                    // annoncer vaut mieux que d'inventer un camp.
                    if signe != 0 {
                        sorties.push(Bascule::RegimeGamma {
                            gex: etat.gex,
                            negatif: signe < 0,
                        });
                    }
                } else {
                    self.candidat = Some((signe, compte));
                }
            }
        }

        // Les niveaux : un franchissement n'existe que d'un côté franc à
        // l'autre. Entrer dans la bande morte ne bascule rien, en sortir du même
        // côté non plus.
        if let (Some(avant), Some(apres), Some(niveau)) = (self.cote_zero, z, etat.zero_gamma)
            && avant != apres
        {
            sorties.push(Bascule::ZeroGamma {
                niveau,
                vers_le_haut: apres,
            });
        }
        for (avant, apres, niveau, nom) in [
            (self.cote_call, c, etat.call_wall, "call wall"),
            (self.cote_put, p, etat.put_wall, "put wall"),
        ] {
            if let (Some(avant), Some(apres), Some(niveau)) = (avant, apres, niveau)
                && avant != apres
            {
                sorties.push(Bascule::Mur {
                    nom,
                    niveau,
                    vers_le_haut: apres,
                });
            }
        }

        // Un côté indéterminé n'écrase pas le dernier côté connu : sinon un
        // passage dans la bande morte ferait oublier d'où l'on venait, et la
        // sortie de bande ressemblerait à un franchissement.
        self.cote_zero = z.or(self.cote_zero);
        self.cote_call = c.or(self.cote_call);
        self.cote_put = p.or(self.cote_put);
        sorties
    }
}

/// La charge à remettre au programme d'envoi, au format que Discord attend.
///
/// **Le collecteur ne connaît pas Discord**, et ne devrait pas : où part une
/// alerte est une affaire de déploiement, pas de collecte. Il produit une charge
/// et l'écrit sur l'entrée standard d'un programme que l'on configure — un
/// `curl` d'une ligne pour Discord, autre chose pour Telegram ou un courriel.
///
/// Le format Discord est retenu comme défaut parce que c'est celui qui sert
/// aujourd'hui, et parce qu'une charge déjà formée évite d'avoir à installer un
/// outil de transformation JSON sur la machine qui veille.
///
/// Aucune E/S ici : la fonction rend une chaîne, elle n'envoie rien.
pub fn charge_json(
    bascule: &Bascule,
    etat: &Etat,
    produit: &str,
    horizon: i64,
    instant: &str,
) -> String {
    let couleur = match bascule {
        Bascule::RegimeGamma { negatif: true, .. } => 0xE0_2B_2B,
        Bascule::RegimeGamma { negatif: false, .. } => 0x1F_6B_4A,
        Bascule::ZeroGamma { .. } => 0xE0_8A_2B,
        Bascule::Mur { .. } => 0x2B_7B_E0,
    };
    let mut lignes = vec![
        format!("spot {:.2}", etat.spot),
        format!("GEX {:+.0}", etat.gex),
    ];
    match bascule {
        Bascule::ZeroGamma { niveau, .. } => {
            lignes.push(format!("zero gamma {niveau:.2}"));
            // La zone vient du SIGNE DU GEX, jamais du sens du franchissement.
            //
            // La tentation serait de dire « franchi par le haut, donc on passe en
            // zone positive ». C'est faux dès que les ailes sont chargées : le
            // profil croise zéro plusieurs fois, on n'en retient qu'un, et
            // au-dessus de celui-là le gamma peut très bien rester négatif. Le
            // dépôt le dit déjà de son côté écran — « c'est faux quand il y en a
            // trois » — et c'est précisément dans ce cas-là qu'une alerte compte.
            //
            // Le signe du GEX, lui, ne se déduit pas : il est mesuré.
            lignes.push(if etat.gex < 0.0 {
                "zone NÉGATIVE — les couvertures amplifient".to_string()
            } else {
                "zone POSITIVE — les couvertures amortissent".to_string()
            });
        }
        Bascule::Mur { nom, niveau, .. } => lignes.push(format!("{nom} {niveau:.2}")),
        Bascule::RegimeGamma { .. } => {
            if let Some(z) = etat.zero_gamma {
                lignes.push(format!("zero gamma {z:.2}"));
            }
        }
    }
    lignes.push(format!("horizon <= {horizon} j  |  {instant}"));

    serde_json::json!({
        "username": "GEX",
        "embeds": [{
            "title": format!("{produit} — {}", bascule.titre()),
            "description": lignes.join("\n"),
            "color": couleur,
        }],
        // Explicite : une donnée recopiée depuis un champ texte ne doit jamais
        // devenir une commande.
        "allowed_mentions": {"parse": []},
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn etat(spot: f64, gex: f64) -> Etat {
        Etat {
            spot,
            gex,
            zero_gamma: Some(29_000.0),
            call_wall: Some(29_500.0),
            put_wall: Some(28_500.0),
        }
    }

    /// Sans cela, un collecteur qui démarre en gamma négatif annoncerait un
    /// basculement qui n'a pas eu lieu — tous les matins.
    #[test]
    fn le_premier_passage_n_annonce_rien() {
        let mut v = Veilleuse::default();
        assert!(v.observer(&etat(29_100.0, -1e8)).is_empty());
    }

    /// Un aller-retour du signe ne doit rien déclencher : c'est du bruit, et
    /// c'est le cas le plus fréquent près de zéro.
    #[test]
    fn un_signe_qui_ne_tient_pas_ne_declenche_rien() {
        let mut v = Veilleuse::neuve(3, MARGE_DEFAUT);
        v.observer(&etat(29_100.0, -1e8));
        assert!(v.observer(&etat(29_100.0, 1e8)).is_empty(), "1er passage");
        assert!(v.observer(&etat(29_100.0, 1e8)).is_empty(), "2e passage");
        // Il repasse négatif avant d'avoir tenu : rien n'a jamais été annoncé.
        assert!(v.observer(&etat(29_100.0, -1e8)).is_empty());
        assert!(v.observer(&etat(29_100.0, -1e8)).is_empty());
    }

    #[test]
    fn un_signe_qui_tient_est_annonce_une_fois() {
        let mut v = Veilleuse::neuve(3, MARGE_DEFAUT);
        v.observer(&etat(29_100.0, -1e8));
        v.observer(&etat(29_100.0, 1e8));
        v.observer(&etat(29_100.0, 1e8));
        let sorties = v.observer(&etat(29_100.0, 1e8));
        assert_eq!(
            sorties,
            vec![Bascule::RegimeGamma {
                gex: 1e8,
                negatif: false
            }]
        );
        // Et il ne se répète pas au passage suivant.
        assert!(v.observer(&etat(29_100.0, 1e8)).is_empty());
    }

    /// Un GEX exactement nul n'est pas un régime : inventer un camp serait pire
    /// que de se taire.
    #[test]
    fn un_gex_nul_n_annonce_aucun_regime() {
        let mut v = Veilleuse::neuve(1, MARGE_DEFAUT);
        v.observer(&etat(29_100.0, -1e8));
        assert!(v.observer(&etat(29_100.0, 0.0)).is_empty());
    }

    /// **Le garde-fou qui compte le plus.** Un prix qui longe un niveau le
    /// franchirait à chaque cotation, et une séance produirait des dizaines
    /// d'alertes décrivant la même chose.
    #[test]
    fn un_prix_qui_longe_un_niveau_ne_le_franchit_pas() {
        let mut v = Veilleuse::neuve(3, 0.001); // 29 points sur 29 000
        v.observer(&etat(29_100.0, -1e8));
        // Il descend dans la bande morte, puis en ressort du même côté.
        for spot in [29_020.0, 28_990.0, 29_010.0, 29_100.0] {
            assert!(
                v.observer(&etat(spot, -1e8)).is_empty(),
                "spot {spot} n'aurait pas dû déclencher"
            );
        }
    }

    #[test]
    fn un_franchissement_franc_est_annonce() {
        let mut v = Veilleuse::neuve(3, 0.001);
        v.observer(&etat(29_100.0, -1e8)); // au-dessus du zero gamma
        let sorties = v.observer(&etat(28_800.0, -1e8)); // franchement en dessous
        assert!(sorties.contains(&Bascule::ZeroGamma {
            niveau: 29_000.0,
            vers_le_haut: false
        }));
    }

    #[test]
    fn les_deux_murs_se_franchissent_dans_les_deux_sens() {
        let mut v = Veilleuse::neuve(3, 0.001);
        v.observer(&etat(29_000.0, -1e8));
        let haut = v.observer(&etat(29_800.0, -1e8));
        assert!(haut.contains(&Bascule::Mur {
            nom: "call wall",
            niveau: 29_500.0,
            vers_le_haut: true
        }));
        let bas = v.observer(&etat(28_200.0, -1e8));
        assert!(bas.contains(&Bascule::Mur {
            nom: "put wall",
            niveau: 28_500.0,
            vers_le_haut: false
        }));
    }

    /// Un niveau absent ne bascule rien, et ne fait pas oublier les autres.
    #[test]
    fn un_niveau_absent_ne_declenche_ni_n_efface() {
        let mut v = Veilleuse::neuve(3, 0.001);
        v.observer(&etat(29_100.0, -1e8));
        let mut sans = etat(29_100.0, -1e8);
        sans.zero_gamma = None;
        assert!(v.observer(&sans).is_empty());
        // Le zero gamma revient, et le prix est passé dessous : le côté d'avant
        // n'a pas été perdu, donc le franchissement se voit.
        let sorties = v.observer(&etat(28_800.0, -1e8));
        assert!(
            sorties
                .iter()
                .any(|b| matches!(b, Bascule::ZeroGamma { .. }))
        );
    }

    #[test]
    fn le_titre_dit_le_sens() {
        assert!(
            Bascule::RegimeGamma {
                gex: -1e8,
                negatif: true
            }
            .titre()
            .contains("amplifient")
        );
        assert!(
            Bascule::Mur {
                nom: "put wall",
                niveau: 1.0,
                vers_le_haut: false
            }
            .titre()
            .contains("PAR LE BAS")
        );
    }

    #[test]
    fn la_charge_porte_le_niveau_qui_a_bascule() {
        let e = etat(28_800.0, -1e8);
        let b = Bascule::ZeroGamma {
            niveau: 29_000.0,
            vers_le_haut: false,
        };
        let charge = charge_json(&b, &e, "NQ", 30, "2026-09-02 14:05");
        let v: serde_json::Value = serde_json::from_str(&charge).expect("JSON valide");
        let titre = v["embeds"][0]["title"].as_str().unwrap();
        assert!(
            titre.starts_with("NQ — ZERO GAMMA FRANCHI PAR LE BAS"),
            "{titre}"
        );
        let corps = v["embeds"][0]["description"].as_str().unwrap();
        assert!(corps.contains("zero gamma 29000.00"), "{corps}");
        assert!(corps.contains("horizon <= 30 j"), "{corps}");
    }

    /// La zone annoncée suit le GEX, pas le sens du franchissement.
    ///
    /// C'est toute la difficulté du zero gamma : le profil croise zéro plusieurs
    /// fois dès que les ailes sont chargées, on n'en retient qu'un, et « franchi
    /// par le haut donc zone positive » devient faux. Ce test prend exactement ce
    /// cas — un franchissement vers le haut alors que le GEX reste négatif — et
    /// exige que le message dise NÉGATIVE. Sans lui, la déduction commode
    /// reviendrait un jour dans le code sans que rien ne l'arrête.
    #[test]
    fn la_zone_suit_le_gex_et_non_le_sens_du_franchissement() {
        let monte_mais_negatif = charge_json(
            &Bascule::ZeroGamma {
                niveau: 29_000.0,
                vers_le_haut: true,
            },
            &etat(29_200.0, -1e8),
            "NQ",
            30,
            "2026-09-02 14:05",
        );
        let v: serde_json::Value = serde_json::from_str(&monte_mais_negatif).unwrap();
        let corps = v["embeds"][0]["description"].as_str().unwrap();
        assert!(corps.contains("zone NÉGATIVE"), "{corps}");
        assert!(corps.contains("amplifient"), "{corps}");

        let descend_mais_positif = charge_json(
            &Bascule::ZeroGamma {
                niveau: 29_000.0,
                vers_le_haut: false,
            },
            &etat(28_800.0, 1e8),
            "NQ",
            30,
            "2026-09-02 14:05",
        );
        let v: serde_json::Value = serde_json::from_str(&descend_mais_positif).unwrap();
        let corps = v["embeds"][0]["description"].as_str().unwrap();
        assert!(corps.contains("zone POSITIVE"), "{corps}");
        assert!(corps.contains("amortissent"), "{corps}");
    }

    /// Une donnée recopiée depuis un champ texte ne doit jamais devenir une
    /// commande sur le salon qui la reçoit.
    #[test]
    fn aucune_mention_n_est_interpretee() {
        let charge = charge_json(
            &Bascule::RegimeGamma {
                gex: -1e8,
                negatif: true,
            },
            &etat(29_000.0, -1e8),
            "@everyone",
            30,
            "2026-09-02 14:05",
        );
        let v: serde_json::Value = serde_json::from_str(&charge).unwrap();
        assert_eq!(v["allowed_mentions"]["parse"].as_array().unwrap().len(), 0);
    }

    /// Zéro confirmation ferait du premier passage sa propre confirmation.
    #[test]
    fn le_seuil_de_confirmation_ne_descend_pas_sous_un() {
        let mut v = Veilleuse::neuve(0, MARGE_DEFAUT);
        v.observer(&etat(29_100.0, -1e8));
        assert_eq!(v.observer(&etat(29_100.0, 1e8)).len(), 1);
    }
}
