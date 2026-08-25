//! Le moteur : d'une chaîne d'options à tous les chiffres d'une séance.
//!
//! Fonction pure, sans réseau ni disque ni graphique — c'est le point d'entrée
//! testable du projet, et la raison pour laquelle `gex-core` ne déclare aucune
//! dépendance capable d'ouvrir quoi que ce soit.
//!
//! La convention de signe est celle de la littérature : les dealers sont **longs
//! les calls et shorts les puts**. Un GEX total négatif signifie donc qu'ils
//! doivent vendre quand le marché baisse — le régime qui amplifie les mouvements.

use chrono::{Datelike, Weekday};

use crate::black76::Sens;
use crate::chaine::{Chaine, ChaineInvalide, Ligne};
use crate::greeks::{Position, charm_exposition, gamma_exposition, vanna_exposition};
use crate::temps::{Convention, EcheanceNy, InstantReleve, temps_restant};

/// D'où vient le gamma qui sert au GEX par strike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceGamma {
    /// Recalculé depuis la volatilité implicite. Seul choix cohérent de bout en
    /// bout : le profil ne PEUT être que recalculé, un gamma publié n'existant
    /// qu'au spot du moment et pas aux niveaux hypothétiques.
    #[default]
    Iv,
    /// Tel que diffusé par la source. IB le publie, ce que ni le CME ni Databento
    /// ne faisaient — c'est ce qui rend la confrontation possible sur du future.
    Publie,
}

/// Ce que devient la volatilité quand le spot bouge, dans le profil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegimeVol {
    /// Chaque contrat garde son IV. Le prix glisse le long du skew existant, ce
    /// qui fait monter mécaniquement la vol à la monnaie quand le spot baisse.
    /// C'est l'hypothèse de la littérature.
    #[default]
    StickyStrike,
    /// Le smile est figé en monnaie et se translate avec le spot. L'effet de
    /// levier disparaît. Le décalage vaut zéro au spot courant et croît avec la
    /// distance : ce régime remodèle les ailes, pas le centre.
    StickyMoneyness,
}

/// Les réglages d'une analyse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Parametres {
    /// Multiplicateur du contrat.
    pub taille_contrat: f64,
    /// Horizon d'échéance en jours calendaires. `None` garde toute la chaîne.
    pub dte_max: Option<i64>,
    /// Échéances à moins de N jours exclues.
    pub dte_min: i64,
    /// Demi-plage du profil autour du spot. 0,2 = +/-20 %.
    pub plage: f64,
    /// Demi-plage de recherche des murs gamma.
    pub plage_murs: f64,
    /// Demi-plage des murs en open interest brut, plus large : ces
    /// concentrations-là sont plus lointaines que les murs gamma, qui restent
    /// attirés vers la monnaie.
    pub plage_murs_oi: f64,
    /// D'où vient le gamma.
    pub source_gamma: SourceGamma,
    /// Comment se mesure le temps restant.
    pub convention: Convention,
    /// Comportement de l'IV dans le profil.
    pub regime_vol: RegimeVol,
    /// Nombre de niveaux du profil.
    pub niveaux: usize,
}

impl Default for Parametres {
    fn default() -> Self {
        Parametres {
            taille_contrat: 1.0,
            dte_max: Some(30),
            dte_min: 0,
            plage: 0.2,
            plage_murs: 0.15,
            plage_murs_oi: 0.30,
            source_gamma: SourceGamma::default(),
            convention: Convention::Heures,
            regime_vol: RegimeVol::default(),
            niveaux: 60,
        }
    }
}

/// Une ligne de chaîne, augmentée de ses expositions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LigneExposee {
    /// La ligne d'origine.
    pub ligne: Ligne,
    /// Temps restant, en années.
    pub t: f64,
    /// Jours calendaires jusqu'à l'échéance.
    pub dte: i64,
    /// GEX du côté call, positif.
    pub call_gex: f64,
    /// GEX du côté put, négatif — les dealers en sont shorts.
    pub put_gex: f64,
    /// GEX net de la ligne.
    pub gex: f64,
    /// GEX net si le gamma vient de l'IV, quelle que soit la source retenue.
    pub gex_iv: f64,
    /// GEX net si le gamma vient de la source. Zéro quand elle n'en publie pas.
    pub gex_publie: f64,
    /// Charm net, en dollars de delta par jour.
    pub charm: f64,
    /// Vanna nette, en dollars de delta par point de vol.
    pub vanna: f64,
    /// Delta dollar du book dealer.
    pub delta: f64,
    /// Vega dollar du book dealer.
    pub vega: f64,
}

/// Les expositions cumulées d'un strike, toutes échéances confondues.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AgregatStrike {
    /// Le strike.
    pub strike: f64,
    /// GEX du côté call.
    pub call_gex: f64,
    /// GEX du côté put.
    pub put_gex: f64,
    /// GEX net.
    pub gex: f64,
    /// Open interest des calls.
    pub call_oi: f64,
    /// Open interest des puts.
    pub put_oi: f64,
}

/// Les quatre murs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Murs {
    /// Strike concentrant le plus de gamma call au-dessus du spot : la résistance.
    pub call: Option<f64>,
    /// Strike concentrant le plus de gamma put sous le spot : le support.
    pub put: Option<f64>,
    /// Plus gros open interest call au-dessus du spot.
    pub call_oi: Option<f64>,
    /// Plus gros open interest put sous le spot.
    pub put_oi: Option<f64>,
}

/// Les deux estimateurs du GEX, et leur écart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EcartGamma {
    /// Total avec le gamma recalculé depuis l'IV.
    pub iv: f64,
    /// Total avec le gamma publié par la source.
    pub publie: f64,
    /// Écart relatif, rapporté au publié.
    pub relatif: f64,
}

/// Le poids des échéances à 0-1 jour.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PartCourtes {
    /// Part du GEX total, en valeur absolue.
    pub gex: Option<f64>,
    /// Part du charm total.
    pub charm: Option<f64>,
}

/// Tout ce qu'une séance produit.
#[derive(Debug, Clone, PartialEq)]
pub struct Analyse {
    /// Le prix du sous-jacent.
    pub spot: f64,
    /// L'instant du relevé.
    pub releve: InstantReleve,
    /// Le multiplicateur appliqué.
    pub taille_contrat: f64,
    /// Les lignes retenues, avec leurs expositions.
    pub lignes: Vec<LigneExposee>,
    /// L'agrégat par strike, trié.
    pub par_strike: Vec<AgregatStrike>,
    /// Les niveaux de spot du profil.
    pub niveaux: Vec<f64>,
    /// Le profil de gamma, toutes échéances.
    pub profil: Vec<f64>,
    /// GEX total.
    pub gex: f64,
    /// Charm total.
    pub charm: f64,
    /// Vanna totale.
    pub vanna: f64,
    /// Delta dollar total.
    pub delta: f64,
    /// Vega dollar total.
    pub vega: f64,
    /// Le croisement de zéro retenu : le plus proche du spot.
    pub zero_gamma: Option<f64>,
    /// Tous les croisements trouvés.
    pub croisements: Vec<f64>,
    /// Les murs.
    pub murs: Murs,
    /// L'écart entre les deux estimateurs de gamma.
    pub ecart_gamma: Option<EcartGamma>,
    /// Le poids des 0-1 DTE.
    pub part_courtes: PartCourtes,
}

/// Jours calendaires jusqu'à l'échéance.
///
/// Les deux bornes sont ramenées à leur date. Depuis que l'échéance porte son
/// heure de règlement et la valorisation son heure de collecte, soustraire les
/// instants ferait qu'une échéance de demain matin, vue ce soir, compterait zéro
/// jour : « 1DTE » cesserait de vouloir dire « expire demain ».
pub fn dte_calendaire(echeance: EcheanceNy, releve: InstantReleve) -> i64 {
    (echeance.0.date() - releve.0.date()).num_days()
}

/// Le troisième vendredi du mois : l'échéance mensuelle.
fn est_troisieme_vendredi(echeance: EcheanceNy) -> bool {
    let d = echeance.0.date();
    d.weekday() == Weekday::Fri && (15..=21).contains(&d.day())
}

/// Ne garde que les échéances exploitables.
///
/// Deux conditions, et la seconde a été apprise en production. La fenêtre en
/// jours d'abord : sans elle, les échéances lointaines — strikes ronds à très
/// gros OI — dominent les murs et tirent le zero gamma alors qu'elles ne
/// produisent aucun flux de couverture à court terme.
///
/// Puis l'échéance réellement passée. Un contrat déjà réglé verrait son T planché
/// à une minute, et le gamma variant en 1/racine(T), ce mort dominerait la chaîne
/// entière — c'est exactement ce qui gonflait le charm d'un facteur trois.
pub fn filtre_echeances(
    chaine: &Chaine,
    dte_max: Option<i64>,
    dte_min: i64,
) -> Result<Chaine, ChaineInvalide> {
    let releve = chaine.releve;
    let plancher = dte_min.max(0);
    chaine.filtrer(|l| {
        let dte = dte_calendaire(l.echeance, releve);
        dte >= plancher
            && dte_max.is_none_or(|max| dte <= max)
            && l.echeance.en_utc() > releve.0
    })
}

/// Ajoute à chaque ligne ses expositions.
pub fn expositions(chaine: &Chaine, params: &Parametres) -> Vec<LigneExposee> {
    let spot = chaine.spot;
    let cs = params.taille_contrat;
    let jours_par_an = params.convention.jours_par_an();

    chaine
        .lignes()
        .iter()
        .map(|l| {
            let t = temps_restant(l.echeance, chaine.releve, params.convention);
            let pc = Position::nouvelle(spot, l.strike, l.call.iv, t, l.call.open_interest, cs);
            let pp = Position::nouvelle(spot, l.strike, l.put.iv, t, l.put.open_interest, cs);

            let iv_call = gamma_exposition(&pc, Sens::Call);
            let iv_put = gamma_exposition(&pp, Sens::Put);
            // Le gamma publié est unitaire : on lui applique la même mise à
            // l'échelle que celle intégrée à gamma_exposition.
            let echelle = cs * spot * spot * 0.01;
            let pub_call = l.call.gamma * l.call.open_interest * echelle;
            let pub_put = l.put.gamma * l.put.open_interest * echelle;

            let (call_gex, put_gex) = match params.source_gamma {
                SourceGamma::Iv => (iv_call, -iv_put),
                SourceGamma::Publie => (pub_call, -pub_put),
            };

            let charm = charm_exposition(&pc, jours_par_an) - charm_exposition(&pp, jours_par_an);
            let vanna = vanna_exposition(&pc) - vanna_exposition(&pp);

            LigneExposee {
                ligne: *l,
                t,
                dte: dte_calendaire(l.echeance, chaine.releve),
                call_gex,
                put_gex,
                gex: call_gex + put_gex,
                gex_iv: iv_call - iv_put,
                gex_publie: pub_call - pub_put,
                charm,
                vanna,
                // Le delta des puts étant négatif, en être short ajoute du delta
                // positif : d'où la soustraction, qui n'est pas une faute de signe.
                delta: (l.call.delta * l.call.open_interest - l.put.delta * l.put.open_interest)
                    * cs
                    * spot,
                vega: (l.call.vega * l.call.open_interest - l.put.vega * l.put.open_interest) * cs,
            }
        })
        .collect()
}

/// Cumule les expositions par strike, toutes échéances confondues.
pub fn par_strike(lignes: &[LigneExposee]) -> Vec<AgregatStrike> {
    let mut agreges: Vec<AgregatStrike> = Vec::new();
    let mut tries: Vec<&LigneExposee> = lignes.iter().collect();
    tries.sort_by(|a, b| {
        a.ligne
            .strike
            .partial_cmp(&b.ligne.strike)
            .expect("strike fini")
    });
    for l in tries {
        let strike = l.ligne.strike;
        match agreges.last_mut() {
            Some(dernier) if dernier.strike == strike => {
                dernier.call_gex += l.call_gex;
                dernier.put_gex += l.put_gex;
                dernier.gex += l.gex;
                dernier.call_oi += l.ligne.call.open_interest;
                dernier.put_oi += l.ligne.put.open_interest;
            }
            _ => agreges.push(AgregatStrike {
                strike,
                call_gex: l.call_gex,
                put_gex: l.put_gex,
                gex: l.gex,
                call_oi: l.ligne.call.open_interest,
                put_oi: l.ligne.put.open_interest,
            }),
        }
    }
    agreges
}

/// Les quatre murs.
///
/// Chaque mur est cherché du bon côté du spot. Sans cette contrainte les deux
/// tombent sur le strike à la monnaie — le gamma unitaire y est maximal, ce qui
/// suffit à battre des strikes dix fois plus chargés en open interest — et le
/// résultat n'est plus qu'une paraphrase du spot.
pub fn murs(par_strike: &[AgregatStrike], spot: f64, params: &Parametres) -> Murs {
    // Un GEX call nul, ou un GEX put non négatif, veut dire qu'il n'y a pas de
    // mur. Prendre l'extremum quand même rendrait le premier strike de la bande,
    // ce qui n'a aucun sens.
    fn extremum(
        candidats: impl Iterator<Item = (f64, f64)>,
        cherche_maximum: bool,
    ) -> Option<f64> {
        let mut meilleur: Option<(f64, f64)> = None;
        for (strike, valeur) in candidats {
            let mieux = match meilleur {
                None => true,
                Some((_, v)) => {
                    if cherche_maximum {
                        valeur > v
                    } else {
                        valeur < v
                    }
                }
            };
            if mieux {
                meilleur = Some((strike, valeur));
            }
        }
        meilleur.and_then(|(strike, valeur)| {
            let significatif = if cherche_maximum {
                valeur > 0.0
            } else {
                valeur < 0.0
            };
            significatif.then_some(strike)
        })
    }

    let dans = |demi: f64, a: &AgregatStrike| {
        a.strike >= (1.0 - demi) * spot && a.strike <= (1.0 + demi) * spot
    };

    Murs {
        call: extremum(
            par_strike
                .iter()
                .filter(|a| dans(params.plage_murs, a) && a.strike >= spot)
                .map(|a| (a.strike, a.call_gex)),
            true,
        ),
        put: extremum(
            par_strike
                .iter()
                .filter(|a| dans(params.plage_murs, a) && a.strike <= spot)
                .map(|a| (a.strike, a.put_gex)),
            false,
        ),
        call_oi: extremum(
            par_strike
                .iter()
                .filter(|a| dans(params.plage_murs_oi, a) && a.strike >= spot)
                .map(|a| (a.strike, a.call_oi)),
            true,
        ),
        put_oi: extremum(
            par_strike
                .iter()
                .filter(|a| dans(params.plage_murs_oi, a) && a.strike <= spot)
                .map(|a| (a.strike, a.put_oi)),
            true,
        ),
    }
}

/// Demi-plage de monnaie sur laquelle le skew est ajusté.
///
/// Au-delà, les ailes se relèvent et une droite n'y décrit plus rien.
const BANDE_SKEW: f64 = 0.15;
/// En deçà, un ajustement au premier degré ne veut rien dire.
const POINTS_SKEW_MINIMUM: usize = 3;

/// Pente du smile, dIV / d(ln K/S), une valeur par échéance.
///
/// Négative presque partout : c'est le skew, les puts hors de la monnaie se
/// paient plus cher que les calls. Une échéance trop pauvre en strikes reçoit une
/// pente nulle, ce qui la ramène au régime sticky-strike plutôt que de lui
/// inventer un skew.
pub fn pente_skew(lignes: &[LigneExposee], echeance: EcheanceNy, spot: f64) -> f64 {
    let points: Vec<(f64, f64)> = lignes
        .iter()
        .filter(|l| l.ligne.echeance == echeance)
        .filter_map(|l| {
            let m = (l.ligne.strike / spot).ln();
            // Les contrats très dans la monnaie sortent avec une IV nulle :
            // les exclure plutôt que de tirer la moyenne vers zéro.
            let ivs: Vec<f64> = [l.ligne.call.iv, l.ligne.put.iv]
                .into_iter()
                .filter(|v| *v > 0.0)
                .collect();
            if ivs.is_empty() || !m.is_finite() || m.abs() > BANDE_SKEW {
                return None;
            }
            Some((m, ivs.iter().sum::<f64>() / ivs.len() as f64))
        })
        .collect();

    if points.len() < POINTS_SKEW_MINIMUM {
        return 0.0;
    }
    let n = points.len() as f64;
    let somme_m: f64 = points.iter().map(|(m, _)| m).sum();
    let somme_v: f64 = points.iter().map(|(_, v)| v).sum();
    let somme_mv: f64 = points.iter().map(|(m, v)| m * v).sum();
    let somme_mm: f64 = points.iter().map(|(m, _)| m * m).sum();
    let denominateur = n * somme_mm - somme_m * somme_m;
    if denominateur.abs() < f64::EPSILON {
        return 0.0;
    }
    (n * somme_mv - somme_m * somme_v) / denominateur
}

/// Le profil de gamma : GEX net à chaque niveau de spot hypothétique.
///
/// Le gamma y est nécessairement recalculé depuis l'IV — à un niveau
/// hypothétique, aucun gamma publié n'existe.
pub fn profil_gamma(
    lignes: &[LigneExposee],
    niveaux: &[f64],
    spot: f64,
    params: &Parametres,
) -> Vec<f64> {
    // La pente ne dépend que de l'échéance : la calculer une fois par ligne
    // referait le même ajustement des dizaines de fois.
    let pentes: Vec<(EcheanceNy, f64)> = if params.regime_vol == RegimeVol::StickyMoneyness {
        let mut vues: Vec<EcheanceNy> = lignes.iter().map(|l| l.ligne.echeance).collect();
        vues.sort();
        vues.dedup();
        vues.into_iter()
            .map(|e| (e, pente_skew(lignes, e, spot)))
            .collect()
    } else {
        Vec::new()
    };

    niveaux
        .iter()
        .map(|&niveau| {
            lignes
                .iter()
                .map(|l| {
                    let (iv_call, iv_put) = match params.regime_vol {
                        RegimeVol::StickyStrike => (l.ligne.call.iv, l.ligne.put.iv),
                        RegimeVol::StickyMoneyness => {
                            let pente = pentes
                                .iter()
                                .find(|(e, _)| *e == l.ligne.echeance)
                                .map(|(_, p)| *p)
                                .unwrap_or(0.0);
                            // ln(K/S') - ln(K/S0) = ln(S0/S') : le décalage ne
                            // dépend pas du strike, ce qui rend ce régime aussi
                            // économe que l'autre.
                            let decalage = pente * (spot / niveau).ln();
                            // Une vol négative n'a pas de gamma, elle a un NaN.
                            // Et un contrat sans IV publiée reste sans IV : on ne
                            // lui en fabrique pas une.
                            let ajuste = |iv: f64| {
                                if iv > 0.0 {
                                    (iv + decalage).max(1e-4)
                                } else {
                                    0.0
                                }
                            };
                            (ajuste(l.ligne.call.iv), ajuste(l.ligne.put.iv))
                        }
                    };
                    let pc = Position::nouvelle(
                        niveau,
                        l.ligne.strike,
                        iv_call,
                        l.t,
                        l.ligne.call.open_interest,
                        params.taille_contrat,
                    );
                    let pp = Position::nouvelle(
                        niveau,
                        l.ligne.strike,
                        iv_put,
                        l.t,
                        l.ligne.put.open_interest,
                        params.taille_contrat,
                    );
                    gamma_exposition(&pc, Sens::Call) - gamma_exposition(&pp, Sens::Put)
                })
                .sum()
        })
        .collect()
}

/// Tous les niveaux où le gamma total change de signe, interpolés linéairement.
pub fn croisements_zero(niveaux: &[f64], profil: &[f64]) -> Vec<f64> {
    niveaux
        .windows(2)
        .zip(profil.windows(2))
        .filter_map(|(x, y)| {
            // Deux zéros consécutifs : rien à interpoler.
            if y[0].signum() == y[1].signum() || y[1] == y[0] {
                return None;
            }
            Some(x[1] - (x[1] - x[0]) * y[1] / (y[1] - y[0]))
        })
        .collect()
}

/// Le croisement retenu : le plus proche du spot.
///
/// Quand le profil croise plusieurs fois — ce qui arrive dès que les ailes sont
/// bruyantes — c'est lui qui délimite le régime dans lequel le marché se trouve
/// effectivement. Prendre le premier croisement de la fenêtre, donc le plus bas,
/// annonçait 91,5 sur un profil croisant en 91, 94 et 106 avec un spot à 100,
/// alors que la bascule se joue à 106.
pub fn zero_gamma(croisements: &[f64], spot: f64) -> Option<f64> {
    croisements
        .iter()
        .copied()
        .min_by(|a, b| {
            (a - spot)
                .abs()
                .partial_cmp(&(b - spot).abs())
                .expect("croisement fini")
        })
}

/// Part du GEX et du charm portée par les échéances à 0-1 jour.
///
/// Le gamma publié et celui recalculé s'accordent au-delà de quelques jours mais
/// divergent violemment sur les 0-1 DTE : près de l'échéance le gamma explose et
/// dépend du spot à la minute, que des données différées ne donnent pas. On ne
/// corrige pas, on signale.
pub fn part_courtes(lignes: &[LigneExposee], seuil_jours: i64) -> PartCourtes {
    let part = |extrait: fn(&LigneExposee) -> f64| {
        let total: f64 = lignes.iter().map(extrait).sum();
        if total == 0.0 {
            return None;
        }
        let proches: f64 = lignes
            .iter()
            .filter(|l| l.dte <= seuil_jours)
            .map(extrait)
            .sum();
        Some(proches.abs() / total.abs())
    };
    if !lignes.iter().any(|l| l.dte <= seuil_jours) {
        return PartCourtes::default();
    }
    PartCourtes {
        gex: part(|l| l.gex),
        charm: part(|l| l.charm),
    }
}

/// L'écart entre les deux estimateurs de gamma.
///
/// `None` quand la source ne publie pas de gamma. Au-delà de quelques pour cent,
/// les données ne décrivent plus le même book que le modèle.
pub fn ecart_gamma(lignes: &[LigneExposee]) -> Option<EcartGamma> {
    let publie: f64 = lignes.iter().map(|l| l.gex_publie).sum();
    let iv: f64 = lignes.iter().map(|l| l.gex_iv).sum();
    if !publie.is_finite() || publie == 0.0 {
        return None;
    }
    Some(EcartGamma {
        iv,
        publie,
        relatif: (iv - publie) / publie.abs(),
    })
}

/// Chaîne d'options brute -> tous les chiffres de la séance.
pub fn analyser(chaine: &Chaine, params: &Parametres) -> Result<Analyse, ChaineInvalide> {
    let retenue = filtre_echeances(chaine, params.dte_max, params.dte_min)?;
    let lignes = expositions(&retenue, params);
    let agreges = par_strike(&lignes);

    let spot = chaine.spot;
    let (bas, haut) = ((1.0 - params.plage) * spot, (1.0 + params.plage) * spot);
    let n = params.niveaux.max(2);
    let pas = (haut - bas) / (n - 1) as f64;
    let niveaux: Vec<f64> = (0..n).map(|i| bas + pas * i as f64).collect();

    let profil = profil_gamma(&lignes, &niveaux, spot, params);
    let croisements = croisements_zero(&niveaux, &profil);

    Ok(Analyse {
        spot,
        releve: chaine.releve,
        taille_contrat: params.taille_contrat,
        gex: lignes.iter().map(|l| l.gex).sum(),
        charm: lignes.iter().map(|l| l.charm).sum(),
        vanna: lignes.iter().map(|l| l.vanna).sum(),
        delta: lignes.iter().map(|l| l.delta).sum(),
        vega: lignes.iter().map(|l| l.vega).sum(),
        zero_gamma: zero_gamma(&croisements, spot),
        murs: murs(&agreges, spot, params),
        ecart_gamma: ecart_gamma(&lignes),
        part_courtes: part_courtes(&lignes, 1),
        par_strike: agreges,
        lignes,
        niveaux,
        profil,
        croisements,
    })
}

/// Le profil restreint aux échéances autres que la plus proche.
///
/// Sert à voir ce que la séance donnerait sans le 0DTE, dont les greeks sont les
/// plus instables. Le même calcul sur les échéances autres que la prochaine
/// mensuelle est obtenu avec [`est_troisieme_vendredi`].
pub fn profil_hors_prochaine(
    analyse: &Analyse,
    params: &Parametres,
    mensuelle: bool,
) -> Option<Vec<f64>> {
    let cible = analyse
        .lignes
        .iter()
        .map(|l| l.ligne.echeance)
        .filter(|e| !mensuelle || est_troisieme_vendredi(*e))
        .min()?;
    let restantes: Vec<LigneExposee> = analyse
        .lignes
        .iter()
        .copied()
        .filter(|l| l.ligne.echeance != cible)
        .collect();
    if restantes.is_empty() {
        return None;
    }
    Some(profil_gamma(
        &restantes,
        &analyse.niveaux,
        analyse.spot,
        params,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chaine::{Cote, Ligne};
    use chrono::NaiveDateTime;

    const SPOT: f64 = 29_300.0;
    const TAILLE: f64 = 20.0;
    const ECHEANCES: [&str; 3] = [
        "2026-08-26 16:00:00",
        "2026-08-28 16:00:00",
        "2026-09-18 09:30:00",
    ];

    fn instant(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    /// La même chaîne synthétique que le générateur Python de `fixtures/` :
    /// skew négatif, open interest asymétrique, trois échéances dont une
    /// mensuelle réglée le matin. Construite depuis des paramètres connus, jamais
    /// figée depuis une sortie observée.
    fn chaine_essai() -> Chaine {
        let releve = InstantReleve(instant("2026-08-25 20:30:00"));
        let mut lignes = Vec::new();
        for (i, ech) in ECHEANCES.iter().enumerate() {
            let echeance = EcheanceNy(instant(ech));
            let t = (echeance.0 - releve.0).num_seconds() as f64 / (365.25 * 24.0 * 3600.0);
            let mut k = 28_500.0_f64;
            while k <= 30_100.0 {
                let m = (k / SPOT).ln();
                let iv = 0.20 - 0.15 * m + 0.02 * i as f64;
                let oi_c = 200.0 + 600.0 * ((k - SPOT) / 500.0).max(0.0);
                let oi_p = 200.0 + 900.0 * ((SPOT - k) / 500.0).max(0.0);
                let g = crate::black76::gamma(SPOT, k, iv, t.max(1e-6), 0.0) * 1.03;
                lignes.push(Ligne {
                    echeance,
                    strike: k,
                    call: Cote {
                        iv,
                        gamma: g,
                        delta: 0.5,
                        open_interest: oi_c,
                        vega: 12.0,
                        theta: 0.0,
                    },
                    put: Cote {
                        iv: iv + 0.005,
                        gamma: g,
                        delta: -0.5,
                        open_interest: oi_p,
                        vega: 11.0,
                        theta: 0.0,
                    },
                });
                k += 100.0;
            }
        }
        Chaine::nouvelle(lignes, SPOT, releve).unwrap()
    }

    fn params() -> Parametres {
        Parametres {
            taille_contrat: TAILLE,
            ..Default::default()
        }
    }

    fn proche(obtenu: f64, attendu: f64, tolerance: f64, quoi: &str) {
        let ecart = ((obtenu - attendu) / attendu).abs();
        assert!(
            ecart <= tolerance,
            "{quoi} : {obtenu:e}, Python donne {attendu:e} (écart relatif {ecart:e})"
        );
    }

    // --- Oracle Python, sur exactement cette chaîne (fixtures/chaine-synthetique.json)

    #[test]
    fn les_totaux_egalent_le_moteur_python() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        assert_eq!(a.lignes.len(), 51);
        proche(a.gex, -325_357_778.743_652, 1e-9, "GEX total");
        proche(a.charm, -644_024_957.130_919_8, 1e-9, "charm total");
        proche(a.vanna, 128_832_672.343_955_31, 1e-9, "vanna totale");
        proche(a.delta, 15_470_400_000.0, 1e-12, "delta total");
        proche(a.vega, -962_400.0, 1e-12, "vega total");
    }

    #[test]
    fn le_zero_gamma_egale_le_moteur_python() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        assert_eq!(a.croisements.len(), 1);
        proche(
            a.zero_gamma.unwrap(),
            29_391.207_190_695_957,
            1e-9,
            "zero gamma",
        );
    }

    #[test]
    fn les_murs_egalent_le_moteur_python() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        assert_eq!(a.murs.call, Some(29_600.0));
        assert_eq!(a.murs.put, Some(28_900.0));
        assert_eq!(a.murs.call_oi, Some(30_100.0));
        assert_eq!(a.murs.put_oi, Some(28_500.0));
    }

    #[test]
    fn l_ecart_de_gamma_egale_le_moteur_python() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        let e = a.ecart_gamma.unwrap();
        proche(e.iv, -325_357_778.743_652, 1e-9, "GEX iv");
        proche(e.publie, -331_164_669.103_518_7, 1e-9, "GEX publié");
        proche(e.relatif, 0.017_534_752_048_237_255, 1e-9, "écart relatif");
    }

    #[test]
    fn le_poids_des_courtes_egale_le_moteur_python() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        proche(
            a.part_courtes.gex.unwrap(),
            0.379_985_209_391_870_64,
            1e-9,
            "part du GEX",
        );
        proche(
            a.part_courtes.charm.unwrap(),
            0.624_817_030_722_563_6,
            1e-9,
            "part du charm",
        );
    }

    #[test]
    fn le_profil_egale_le_moteur_python() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        assert_eq!(a.niveaux.len(), 60);
        proche(a.niveaux[0], 23_440.0, 1e-12, "niveau bas");
        proche(a.niveaux[59], 35_160.0, 1e-12, "niveau haut");
        for (i, attendu) in [
            (0usize, -1_257_565.398_330_257_5),
            (15, -74_069_723.824_968_86),
            (30, 29_154_833.129_124_83),
            (45, 29_694_811.881_880_693),
            (59, 3_002_394.238_615_25),
        ] {
            proche(a.profil[i], attendu, 1e-9, &format!("profil[{i}]"));
        }
    }

    // --- Ce que les tests vérifient sans Python

    /// La régression apprise en production : un contrat déjà réglé verrait son T
    /// planché à une minute, et le gamma variant en 1/racine(T), ce mort
    /// dominerait la chaîne entière.
    #[test]
    fn une_echeance_deja_reglee_est_ecartee() {
        let c = chaine_essai();
        // 2026-08-26 16h00 New York = 20h00 UTC. Vu à 20h30 le 26, elle est morte.
        let tard = Chaine::nouvelle(
            c.lignes().to_vec(),
            SPOT,
            InstantReleve(instant("2026-08-26 20:30:00")),
        )
        .unwrap();
        let a = analyser(&tard, &params()).unwrap();
        assert!(
            a.lignes
                .iter()
                .all(|l| l.ligne.echeance.0 != instant("2026-08-26 16:00:00")),
            "l'échéance réglée à 16h doit être écartée à 16h30"
        );
    }

    #[test]
    fn dte_compte_des_jours_pas_du_temps_ecoule() {
        // Demain 9h30, vu ce soir 20h04 : un jour, pas zéro.
        assert_eq!(
            dte_calendaire(
                EcheanceNy(instant("2026-08-26 09:30:00")),
                InstantReleve(instant("2026-08-25 20:04:00"))
            ),
            1
        );
    }

    /// Sans la contrainte de côté, les deux murs tombent sur le strike à la
    /// monnaie et ne sont plus qu'une paraphrase du spot.
    #[test]
    fn les_murs_restent_du_bon_cote_du_spot() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        assert!(a.murs.call.unwrap() >= SPOT);
        assert!(a.murs.put.unwrap() <= SPOT);
        assert!(a.murs.call_oi.unwrap() >= SPOT);
        assert!(a.murs.put_oi.unwrap() <= SPOT);
    }

    /// Un GEX call nul veut dire qu'il n'y a pas de mur : rendre le premier
    /// strike de la bande n'aurait aucun sens.
    #[test]
    fn sans_gamma_il_n_y_a_pas_de_mur() {
        let vides: Vec<AgregatStrike> = (0..5)
            .map(|i| AgregatStrike {
                strike: SPOT + 100.0 * i as f64,
                ..Default::default()
            })
            .collect();
        let m = murs(&vides, SPOT, &params());
        assert_eq!(m.call, None);
        assert_eq!(m.put, None);
    }

    /// La pente doit retrouver le skew injecté : -0,15 par unité de ln(K/S).
    #[test]
    fn la_pente_du_skew_retrouve_le_skew_injecte() {
        let c = chaine_essai();
        let lignes = expositions(&c, &params());
        let pente = pente_skew(&lignes, EcheanceNy(instant(ECHEANCES[0])), SPOT);
        // Les deux côtés sont moyennés, et le put porte +0,005 : la pente reste
        // celle du skew, le décalage constant n'y contribue pas.
        assert!(
            (pente + 0.15).abs() < 1e-9,
            "pente {pente} au lieu de -0,15"
        );
    }

    /// Le régime remodèle les ailes mais ne déplace quasiment pas un zero gamma
    /// proche du spot : le décalage vaut exactement zéro au spot courant.
    #[test]
    fn sticky_moneyness_ne_deplace_pas_le_centre() {
        let c = chaine_essai();
        let strict = analyser(&c, &params()).unwrap();
        let glissant = analyser(
            &c,
            &Parametres {
                regime_vol: RegimeVol::StickyMoneyness,
                ..params()
            },
        )
        .unwrap();
        let (a, b) = (strict.zero_gamma.unwrap(), glissant.zero_gamma.unwrap());
        assert!(
            (a - b).abs() / SPOT < 0.01,
            "zero gamma déplacé de {} points par le régime de vol",
            (a - b).abs()
        );
        // Les ailes, elles, doivent bouger : sinon le régime ne ferait rien.
        assert!(strict.profil[0] != glissant.profil[0]);
    }

    /// Le croisement retenu est le plus proche du spot, pas le plus bas — c'est
    /// lui qui délimite le régime dans lequel le marché se trouve.
    #[test]
    fn le_zero_retenu_est_le_plus_proche_du_spot() {
        assert_eq!(zero_gamma(&[91.0, 94.0, 103.0], 100.0), Some(103.0));
        assert_eq!(zero_gamma(&[], 100.0), None);
    }

    /// À distance égale, le premier l'emporte. Ce n'est pas un choix défendable
    /// en soi — c'est celui que fait `min()` en Python, et les deux moteurs
    /// doivent trancher pareil pour rester comparables. Vérifié contre lui :
    /// sur 91 / 94 / 106 vu de 100, Python rend 94.
    #[test]
    fn a_distance_egale_le_premier_croisement_l_emporte() {
        assert_eq!(zero_gamma(&[91.0, 94.0, 106.0], 100.0), Some(94.0));
    }

    #[test]
    fn les_croisements_sont_interpoles() {
        let niveaux = [100.0, 110.0, 120.0];
        let profil = [-10.0, 10.0, 20.0];
        let c = croisements_zero(&niveaux, &profil);
        assert_eq!(c.len(), 1);
        assert!((c[0] - 105.0).abs() < 1e-12, "croisement en {}", c[0]);
    }

    #[test]
    fn l_agregat_par_strike_cumule_les_echeances() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        assert_eq!(a.par_strike.len(), 17, "17 strikes distincts");
        let total: f64 = a.par_strike.iter().map(|s| s.gex).sum();
        proche(total, a.gex, 1e-12, "somme des strikes vs GEX total");
        // Trois échéances, donc l'open interest d'un strike est triplé
        let premier = a.par_strike[0];
        assert!((premier.call_oi - 3.0 * 200.0).abs() < 1e-9);
    }

    #[test]
    fn un_horizon_trop_court_ne_garde_rien() {
        let r = analyser(
            &chaine_essai(),
            &Parametres {
                dte_max: Some(0),
                dte_min: 0,
                ..params()
            },
        );
        // La plus proche est à un jour : rien ne reste, et c'est une erreur, pas
        // un GEX de zéro.
        assert_eq!(r.unwrap_err(), ChaineInvalide::Vide);
    }

    #[test]
    fn dte_min_ecarte_les_plus_courtes() {
        let a = analyser(
            &chaine_essai(),
            &Parametres {
                dte_min: 2,
                ..params()
            },
        )
        .unwrap();
        assert!(a.lignes.iter().all(|l| l.dte >= 2));
        assert!(a.lignes.len() < 51);
    }

    #[test]
    fn le_troisieme_vendredi_est_reconnu() {
        // 18 septembre 2026 est un vendredi, et le troisième du mois.
        assert!(est_troisieme_vendredi(EcheanceNy(instant(ECHEANCES[2]))));
        // 26 août 2026 est un mercredi.
        assert!(!est_troisieme_vendredi(EcheanceNy(instant(ECHEANCES[0]))));
    }

    #[test]
    fn le_profil_hors_prochaine_retire_une_echeance() {
        let a = analyser(&chaine_essai(), &params()).unwrap();
        let sans = profil_hors_prochaine(&a, &params(), false).unwrap();
        assert_eq!(sans.len(), a.niveaux.len());
        assert!(sans != a.profil, "retirer une échéance doit changer le profil");
    }

    /// Choisir le gamma publié doit changer le GEX, pas le profil : à un niveau
    /// hypothétique, aucun gamma publié n'existe.
    #[test]
    fn la_source_de_gamma_ne_touche_pas_le_profil() {
        let c = chaine_essai();
        let iv = analyser(&c, &params()).unwrap();
        let publie = analyser(
            &c,
            &Parametres {
                source_gamma: SourceGamma::Publie,
                ..params()
            },
        )
        .unwrap();
        assert!(iv.gex != publie.gex, "le GEX doit changer");
        assert_eq!(iv.profil, publie.profil, "le profil ne doit pas changer");
    }
}
