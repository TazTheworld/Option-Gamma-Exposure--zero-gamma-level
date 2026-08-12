/* Interface GEX — SVG construit à la main, sans bibliothèque.
 *
 * Règles de tracé appliquées partout : marques fines, bouts de barre arrondis à
 * 4px du côté opposé à la ligne de base, 2px de surface entre deux barres
 * voisines, grille et axes en filet discret, légende systématique dès deux
 * séries, et une vue tableau pour chaque graphique — aucune valeur n'est
 * accessible uniquement par la couleur ou par le survol.
 */

const COULEURS = {
  serie1: "#2a78d6", serie2: "#eb6834", serie3: "#1baf7a",
  positif: "#2a78d6", negatif: "#e34948",
  encre: "#0b0b0b", encre2: "#52514e", encre3: "#898781",
  grille: "#e1e0d9", axe: "#c3c2b7", surface: "#fcfcfb",
};

const NOMS_PROFILS = {
  "All Expiries": "Toutes échéances",
  "Ex-Next Expiry": "Hors prochaine échéance",
  "Ex-Next Monthly Expiry": "Hors prochaine mensuelle",
};

const NS = "http://www.w3.org/2000/svg";

// ---=== Formatage ===---

const nf = (min, max) => new Intl.NumberFormat("fr-FR", {
  minimumFractionDigits: min, maximumFractionDigits: max,
});

function prix(v, decimales = 2) {
  if (v === null || v === undefined || !isFinite(v)) return "—";
  return nf(decimales, decimales).format(v);
}

/** Choisit milliards ou millions selon l'ordre de grandeur, comme pick_scale(). */
function echelle(valeurs) {
  const pic = Math.max(0, ...valeurs.filter(isFinite).map(Math.abs));
  if (pic >= 1e9) return { facteur: 1e9, unite: "Md $" };
  if (pic >= 1e6) return { facteur: 1e6, unite: "M $" };
  if (pic >= 1e3) return { facteur: 1e3, unite: "k $" };
  return { facteur: 1, unite: "$" };
}

function montant(v, ech, decimales = 2, signe = false) {
  if (v === null || v === undefined || !isFinite(v)) return "—";
  const x = v / ech.facteur;
  const texte = nf(decimales, decimales).format(x);
  return `${signe && x > 0 ? "+" : ""}${texte} ${ech.unite}`;
}

/** Bornes d'axe « rondes » : sans ça les graduations tombent sur des décimales absurdes. */
function graduations(min, max, cible = 5) {
  if (!isFinite(min) || !isFinite(max) || min === max) return [min || 0];
  const brut = (max - min) / cible;
  const magnitude = Math.pow(10, Math.floor(Math.log10(brut)));
  const pas = [1, 2, 2.5, 5, 10].map((m) => m * magnitude)
    .find((p) => p >= brut) || 10 * magnitude;
  const sortie = [];
  for (let t = Math.ceil(min / pas) * pas; t <= max + pas * 1e-9; t += pas) sortie.push(t);
  return sortie;
}

// ---=== Fabrique SVG ===---

function el(nom, attrs = {}, texte = null) {
  const noeud = document.createElementNS(NS, nom);
  for (const [k, v] of Object.entries(attrs)) {
    if (v !== null && v !== undefined) noeud.setAttribute(k, v);
  }
  if (texte !== null) noeud.textContent = texte;
  return noeud;
}

const L = 1000;                                    // largeur du viewBox
const MARGE = { haut: 26, droite: 22, bas: 44, gauche: 74 };

function toile(hauteur) {
  const svg = el("svg", {
    viewBox: `0 0 ${L} ${hauteur}`,
    role: "img",
    preserveAspectRatio: "xMidYMid meet",
  });
  return svg;
}

/** Chemin d'une barre au bout arrondi, l'arrondi étant du côté opposé à la base. */
function cheminBarre(x, yBase, yValeur, largeur, rayon = 4) {
  const versLeHaut = yValeur <= yBase;
  const y = versLeHaut ? yValeur : yBase;
  const h = Math.abs(yBase - yValeur);
  const r = Math.max(0, Math.min(rayon, largeur / 2, h));
  if (h < 0.4) return `M${x} ${yBase}h${largeur}`;   // valeur quasi nulle : un trait
  if (versLeHaut) {
    return `M${x} ${y + h}L${x} ${y + r}Q${x} ${y} ${x + r} ${y}` +
           `L${x + largeur - r} ${y}Q${x + largeur} ${y} ${x + largeur} ${y + r}` +
           `L${x + largeur} ${y + h}Z`;
  }
  return `M${x} ${y}L${x} ${y + h - r}Q${x} ${y + h} ${x + r} ${y + h}` +
         `L${x + largeur - r} ${y + h}Q${x + largeur} ${y + h} ${x + largeur} ${y + h - r}` +
         `L${x + largeur} ${y}Z`;
}

/** Grille horizontale + axes, communs à tous les graphiques. */
function chrome(svg, geo, yTicks, formateY, titreY, xTicks, formateX) {
  const { x0, x1, y0, y1, yEch } = geo;

  // Le titre d'axe se pose EN HAUT À GAUCHE du champ, pas à gauche de l'axe :
  // ancré à droite il sortait du cadre dès que l'unité s'allongeait.
  if (titreY) {
    svg.appendChild(el("text", {
      x: x0, y: y0 - 10, "text-anchor": "start", class: "axe-titre",
    }, titreY));
  }

  for (const t of yTicks) {
    const y = yEch(t);
    svg.appendChild(el("line", { x1: x0, x2: x1, y1: y, y2: y, class: "grille-ligne" }));
    svg.appendChild(el("text", {
      x: x0 - 10, y: y + 4, "text-anchor": "end", class: "axe-texte",
    }, formateY(t)));
  }

  svg.appendChild(el("line", { x1: x0, x2: x1, y1: y1, y2: y1, class: "axe-ligne" }));
  for (const t of xTicks) {
    svg.appendChild(el("text", {
      x: geo.xEch(t), y: y1 + 20, "text-anchor": "middle", class: "axe-texte",
    }, formateX(t)));
  }
}

/* Repères verticaux : spot, zero gamma, murs.
 *
 * Ce ne sont pas des séries : ils ne prennent aucune couleur catégorielle. Le
 * trait est en encre secondaire et l'étiquette est posée dans une pastille pleine,
 * qui porte le contraste — une teinte fine sur fond clair ne le porterait pas.
 * Les pastilles sont empilées verticalement pour ne pas se recouvrir. */
const REPERES = {
  fort:  { trait: COULEURS.encre,  fond: COULEURS.encre,  texte: "#ffffff" },
  moyen: { trait: COULEURS.encre2, fond: "#eceae4",       texte: COULEURS.encre },
  faible:{ trait: COULEURS.axe,    fond: "#f3f2ee",       texte: COULEURS.encre2 },
};

function repere(svg, geo, valeur, ton, libelle, rang = 0) {
  if (valeur === null || valeur === undefined || !isFinite(valeur)) return;
  if (valeur < geo.xMin || valeur > geo.xMax) return;
  const style = REPERES[ton] || REPERES.moyen;
  const x = geo.xEch(valeur);

  svg.appendChild(el("line", {
    x1: x, x2: x, y1: geo.y0, y2: geo.y1,
    stroke: style.trait, class: "repere-ligne",
    "stroke-dasharray": ton === "faible" ? "3 3" : null,
  }));

  const largeur = libelle.length * 6.4 + 14;
  const versLaGauche = x + largeur + 6 > geo.x1;
  const xPastille = versLaGauche ? x - largeur - 5 : x + 5;
  const y = geo.y0 + rang * 21;

  svg.appendChild(el("rect", {
    x: xPastille, y, width: largeur, height: 17, rx: 4, fill: style.fond,
  }));
  svg.appendChild(el("text", {
    x: xPastille + largeur / 2, y: y + 12, "text-anchor": "middle",
    fill: style.texte, class: "repere-texte",
  }, libelle));
}

// ---=== Infobulle ===---

const infobulle = document.getElementById("infobulle");

function montrerInfobulle(evenement, titre, lignes) {
  infobulle.innerHTML = "";
  const t = document.createElement("div");
  t.className = "titre";
  t.textContent = titre;
  infobulle.appendChild(t);

  const dl = document.createElement("dl");
  for (const [cle, valeur, couleur] of lignes) {
    const dt = document.createElement("dt");
    if (couleur) {
      const puce = document.createElement("i");
      puce.style.cssText =
        `display:inline-block;width:9px;height:9px;border-radius:2px;margin-right:6px;background:${couleur}`;
      dt.appendChild(puce);
    }
    dt.appendChild(document.createTextNode(cle));
    const dd = document.createElement("dd");
    dd.textContent = valeur;
    dl.append(dt, dd);
  }
  infobulle.appendChild(dl);
  infobulle.hidden = false;

  const boite = infobulle.getBoundingClientRect();
  let x = evenement.clientX + 16;
  let y = evenement.clientY - boite.height / 2;
  if (x + boite.width > window.innerWidth - 12) x = evenement.clientX - boite.width - 16;
  y = Math.max(12, Math.min(y, window.innerHeight - boite.height - 12));
  infobulle.style.left = `${x}px`;
  infobulle.style.top = `${y}px`;
}

function cacherInfobulle() { infobulle.hidden = true; }

// ---=== Graphique en barres ===---

/**
 * @param series [{cle, nom, couleur, valeurs}] — une ou deux séries.
 * @param diverge true : la couleur suit le signe (amortit / amplifie).
 */
function graphiqueBarres(conteneur, { strikes, series, diverge, spot, reperes, unite,
                                      hauteur = 330, xMin, xMax }) {
  conteneur.innerHTML = "";
  const svg = toile(hauteur);

  const visibles = strikes
    .map((k, i) => ({ k, i }))
    .filter(({ k }) => k >= xMin && k <= xMax);
  if (!visibles.length) {
    conteneur.appendChild(el("text", { x: 20, y: 40, class: "axe-texte" }, "aucun strike dans la plage"));
    return;
  }

  const toutes = series.flatMap((s) => visibles.map(({ i }) => s.valeurs[i] || 0));
  const ech = echelle(toutes);
  const maxAbs = Math.max(1e-9, ...toutes.map((v) => Math.abs(v / ech.facteur)));
  const yMin = Math.min(0, -maxAbs), yMax = Math.max(0, maxAbs);

  const x0 = MARGE.gauche, x1 = L - MARGE.droite;
  const y0 = MARGE.haut, y1 = hauteur - MARGE.bas;
  const xEch = (v) => x0 + ((v - xMin) / (xMax - xMin || 1)) * (x1 - x0);
  const yEch = (v) => y1 - ((v - yMin) / (yMax - yMin || 1)) * (y1 - y0);
  const geo = { x0, x1, y0, y1, xEch, yEch, xMin, xMax };

  const yTicks = graduations(yMin, yMax, 4);
  chrome(svg, geo, yTicks, (t) => nf(0, 2).format(t), unite,
         graduations(xMin, xMax, 6), (t) => nf(0, 0).format(t));

  // Pas réel entre strikes : la largeur suit l'espacement, moins 2px de surface.
  const pas = visibles.length > 1
    ? (x1 - x0) / (visibles.length - 1) : (x1 - x0) / 6;
  const largeur = Math.max(1.5, Math.min(pas - 2, 34));

  const yBase = yEch(0);
  const groupe = el("g");
  for (const s of series) {
    for (const { k, i } of visibles) {
      const v = (s.valeurs[i] || 0) / ech.facteur;
      if (!isFinite(v)) continue;
      const couleur = diverge ? (v >= 0 ? COULEURS.positif : COULEURS.negatif) : s.couleur;
      groupe.appendChild(el("path", {
        d: cheminBarre(xEch(k) - largeur / 2, yBase, yEch(v), largeur),
        fill: couleur, class: "barre", "data-strike": k,
      }));
    }
  }
  svg.appendChild(groupe);

  svg.appendChild(el("line", { x1: x0, x2: x1, y1: yBase, y2: yBase, class: "axe-ligne" }));
  for (const r of reperes || []) repere(svg, geo, r.valeur, r.ton, r.libelle, r.rang || 0);

  // Cibles de survol : toute la hauteur du champ, larges d'un pas (≥ 24px de confort).
  const cibles = el("g");
  for (const { k, i } of visibles) {
    const largeurCible = Math.max(pas, 24);
    const cible = el("rect", {
      x: xEch(k) - largeurCible / 2, y: y0, width: largeurCible, height: y1 - y0,
      class: "cible",
    });
    cible.addEventListener("mousemove", (evenement) => {
      groupe.querySelectorAll(".barre").forEach((b) => {
        b.classList.toggle("active", Number(b.dataset.strike) === k);
      });
      montrerInfobulle(evenement, `Strike ${prix(k, k < 10 ? 4 : 0)}`,
        series.map((s) => {
          const v = s.valeurs[i] || 0;
          const couleur = diverge ? (v >= 0 ? COULEURS.positif : COULEURS.negatif) : s.couleur;
          return [s.nom, montant(v, ech, 2, true), couleur];
        }));
    });
    cible.addEventListener("mouseleave", () => {
      groupe.querySelectorAll(".barre").forEach((b) => b.classList.remove("active"));
      cacherInfobulle();
    });
    cibles.appendChild(cible);
  }
  svg.appendChild(cibles);

  svg.setAttribute("aria-label",
    `${series.map((s) => s.nom).join(" et ")} par strike, de ${prix(xMin, 0)} à ${prix(xMax, 0)}.`);
  conteneur.appendChild(svg);
  return ech;
}

// ---=== Graphique en courbes ===---

function graphiqueCourbes(conteneur, { niveaux, series, reperes, unite, hauteur = 380,
                                       zoneNegative }) {
  conteneur.innerHTML = "";
  const svg = toile(hauteur);

  const toutes = series.flatMap((s) => s.valeurs).filter(isFinite);
  const ech = echelle(toutes);
  const mises = series.map((s) => s.valeurs.map((v) => v / ech.facteur));
  const plates = mises.flat().filter(isFinite);
  let yMin = Math.min(0, ...plates), yMax = Math.max(0, ...plates);
  const marge = (yMax - yMin) * 0.08 || 1;
  yMin -= marge; yMax += marge;

  const xMin = niveaux[0], xMax = niveaux[niveaux.length - 1];
  // Gouttière à droite : les étiquettes de courbe s'y posent, hors du champ.
  // Collées au bord du tracé, elles se superposaient aux courbes qui convergent.
  const x0 = MARGE.gauche, x1 = L - 186;
  const y0 = MARGE.haut, y1 = hauteur - MARGE.bas;
  const xEch = (v) => x0 + ((v - xMin) / (xMax - xMin || 1)) * (x1 - x0);
  const yEch = (v) => y1 - ((v - yMin) / (yMax - yMin || 1)) * (y1 - y0);
  const geo = { x0, x1, y0, y1, xEch, yEch, xMin, xMax };

  // Zone de gamma négatif : teinte très faible, jamais une couleur de série.
  if (zoneNegative !== null && zoneNegative !== undefined && isFinite(zoneNegative)) {
    const fin = Math.max(x0, Math.min(xEch(zoneNegative), x1));
    if (fin > x0) {
      svg.appendChild(el("rect", {
        x: x0, y: y0, width: fin - x0, height: y1 - y0,
        fill: COULEURS.negatif, opacity: 0.055,
      }));
      // Étiquette tenue DANS le bandeau : débordant à droite, elle donnait
      // l'impression que la zone allait plus loin qu'elle ne va.
      if (fin - x0 > 90) {
        svg.appendChild(el("text", {
          x: x0 + 8, y: y1 - 8, class: "axe-texte", fill: COULEURS.negatif,
        }, "gamma négatif"));
      }
    }
  }

  chrome(svg, geo, graduations(yMin, yMax, 4), (t) => nf(0, 2).format(t), unite,
         graduations(xMin, xMax, 6), (t) => nf(0, 0).format(t));
  svg.appendChild(el("line", {
    x1: x0, x2: x1, y1: yEch(0), y2: yEch(0), class: "axe-ligne",
  }));

  series.forEach((s, index) => {
    const d = mises[index]
      .map((v, i) => `${i ? "L" : "M"}${xEch(niveaux[i]).toFixed(2)} ${yEch(v).toFixed(2)}`)
      .join("");
    svg.appendChild(el("path", { d, stroke: s.couleur, class: "courbe" }));
  });

  // Étiquettes directes dans la gouttière, reliées à leur courbe par un trait
  // court. Elles sont poussées vers le bas tant que deux se chevauchent, sinon
  // les trois se superposent là où les courbes se rejoignent.
  const bouts = series.map((s, index) => ({
    nom: s.nom, couleur: s.couleur,
    yCourbe: yEch(mises[index][mises[index].length - 1]),
  })).sort((a, b) => a.yCourbe - b.yCourbe);
  let precedent = -Infinity;
  for (const bout of bouts) {
    const y = Math.max(bout.yCourbe, precedent + 17);
    precedent = y;
    svg.appendChild(el("path", {
      d: `M${x1} ${bout.yCourbe}L${x1 + 10} ${y}`,
      stroke: bout.couleur, fill: "none", "stroke-width": 1, opacity: 0.6,
    }));
    svg.appendChild(el("text", {
      x: x1 + 14, y: y + 4, "text-anchor": "start", fill: bout.couleur, class: "repere-texte",
    }, bout.nom));
  }

  for (const r of reperes || []) repere(svg, geo, r.valeur, r.ton, r.libelle, r.rang || 0);

  // Curseur : une ligne verticale, un point par courbe, une infobulle.
  const curseur = el("g", { opacity: 0 });
  const trait = el("line", { y1: y0, y2: y1, class: "curseur" });
  curseur.appendChild(trait);
  const points = series.map((s) => {
    const p = el("circle", { r: 4.5, fill: s.couleur, class: "point-curseur" });
    curseur.appendChild(p);
    return p;
  });
  svg.appendChild(curseur);

  const capteur = el("rect", { x: x0, y: y0, width: x1 - x0, height: y1 - y0, class: "cible" });
  capteur.addEventListener("mousemove", (evenement) => {
    const boite = svg.getBoundingClientRect();
    const xVue = ((evenement.clientX - boite.left) / boite.width) * L;
    const niveau = xMin + ((xVue - x0) / (x1 - x0)) * (xMax - xMin);
    let i = 0, ecart = Infinity;
    niveaux.forEach((n, j) => {
      const d = Math.abs(n - niveau);
      if (d < ecart) { ecart = d; i = j; }
    });
    const x = xEch(niveaux[i]);
    trait.setAttribute("x1", x); trait.setAttribute("x2", x);
    points.forEach((p, index) => {
      p.setAttribute("cx", x);
      p.setAttribute("cy", yEch(mises[index][i]));
    });
    curseur.setAttribute("opacity", 1);
    montrerInfobulle(evenement, `Sous-jacent à ${prix(niveaux[i], niveaux[i] < 10 ? 4 : 0)}`,
      series.map((s) => [s.nom, montant(s.valeurs[i], ech, 2, true), s.couleur]));
  });
  capteur.addEventListener("mouseleave", () => {
    curseur.setAttribute("opacity", 0);
    cacherInfobulle();
  });
  svg.appendChild(capteur);

  svg.setAttribute("aria-label", "Profil de gamma selon le niveau du sous-jacent.");
  conteneur.appendChild(svg);
  return ech;
}

// ---=== Légendes et tableaux ===---

function legende(conteneur, entrees) {
  conteneur.innerHTML = "";
  for (const { couleur, nom, trait } of entrees) {
    const s = document.createElement("span");
    const i = document.createElement("i");
    i.style.background = couleur;
    if (trait) i.classList.add("trait");
    s.append(i, document.createTextNode(nom));
    conteneur.appendChild(s);
  }
}

function tableau(conteneur, legendeTexte, entetes, lignes) {
  conteneur.innerHTML = "";
  const t = document.createElement("table");
  const cap = document.createElement("caption");
  cap.textContent = legendeTexte;
  t.appendChild(cap);

  const thead = document.createElement("thead");
  const tr = document.createElement("tr");
  for (const h of entetes) {
    const th = document.createElement("th");
    th.scope = "col";
    th.textContent = h;
    tr.appendChild(th);
  }
  thead.appendChild(tr);
  t.appendChild(thead);

  const tbody = document.createElement("tbody");
  for (const ligne of lignes) {
    const tr2 = document.createElement("tr");
    ligne.forEach((cellule, index) => {
      const cel = document.createElement(index === 0 ? "th" : "td");
      if (index === 0) cel.scope = "row";
      cel.textContent = cellule;
      tr2.appendChild(cel);
    });
    tbody.appendChild(tr2);
  }
  t.appendChild(tbody);
  conteneur.appendChild(t);
}

document.addEventListener("click", (evenement) => {
  const bouton = evenement.target.closest(".lien-tableau");
  if (!bouton) return;
  const cible = document.getElementById(bouton.dataset.tableau);
  cible.hidden = !cible.hidden;
  bouton.textContent = cible.hidden ? "Voir le tableau" : "Masquer le tableau";
});

// ---=== Rendu d'un relevé ===---

const $ = (id) => document.getElementById(id);

function tuile(titre, valeur, note, classe = "") {
  const d = document.createElement("div");
  d.className = "tuile";
  const h = document.createElement("h3");
  h.textContent = titre;
  const v = document.createElement("p");
  v.className = `valeur ${valeur === "—" ? "vide" : classe}`;
  v.textContent = valeur;
  d.append(h, v);
  if (note) {
    const n = document.createElement("p");
    n.className = "note";
    n.textContent = note;
    d.appendChild(n);
  }
  return d;
}

function rendre(d) {
  const dec = d.decimals;
  const echTotal = echelle([d.total_gex]);
  const echGrecs = echelle([d.total_charm, d.total_vanna]);
  const positif = d.total_gex >= 0;

  // --- résumé ---
  $("hero-gex").textContent = montant(d.total_gex, echTotal, 2, true);
  $("hero-gex").className = `hero ${positif ? "est-positif" : "est-negatif"}`;
  $("hero-legende").textContent = "par mouvement de 1 % du sous-jacent";
  $("regime-texte").innerHTML = positif
    ? `Gamma <strong>positif</strong> : la couverture des dealers s'oppose au mouvement
       et le comprime. Le sous-jacent cote ${prix(d.spot, dec)}, contre un zero gamma
       à ${prix(d.zero_gamma, dec)}. Tant que le prix reste au-dessus de ce seuil,
       les mouvements ont tendance à s'amortir d'eux-mêmes.`
    : `Gamma <strong>négatif</strong> : la couverture des dealers accompagne le mouvement
       et l'amplifie. Le sous-jacent cote ${prix(d.spot, dec)}, contre un zero gamma
       à ${prix(d.zero_gamma, dec)}. C'est le régime où la volatilité réalisée
       dépasse ce que la vol implicite laisse attendre.`;
  $("resume").hidden = false;

  // --- tuiles ---
  const tuiles = $("tuiles");
  tuiles.innerHTML = "";
  tuiles.append(
    tuile("Sous-jacent", prix(d.spot, dec), `${d.n_strikes} strikes · ${d.n_expiries} échéances`),
    tuile("Zero gamma", prix(d.zero_gamma, dec),
      d.croisements.length > 1 ? `${d.croisements.length} croisements — le plus proche du spot`
                               : "bascule de régime"),
    tuile("Call wall", prix(d.call_wall, dec),
      `open interest : ${prix(d.call_wall_oi, dec)}`),
    tuile("Put wall", prix(d.put_wall, dec),
      `open interest : ${prix(d.put_wall_oi, dec)}`),
    // Ni charm ni vanna ne sont colorés : leur signe n'est pas « bon » ou « mauvais »,
    // il dit seulement dans quel sens les dealers devront couvrir.
    tuile("Charm", montant(d.total_charm, echGrecs, 2, true),
      d.total_charm < 0 ? "de delta par jour — les dealers achètent"
                        : "de delta par jour — les dealers vendent"),
    tuile("Vanna", montant(d.total_vanna, echGrecs, 2, true),
      d.total_vanna >= 0 ? "de delta par point de vol — vendeurs si la vol monte"
                         : "de delta par point de vol — acheteurs si la vol monte"),
  );
  tuiles.hidden = false;

  const reperesCommuns = [
    { valeur: d.spot, ton: "fort", libelle: `Spot ${prix(d.spot, dec)}`, rang: 0 },
    { valeur: d.zero_gamma, ton: "moyen",
      libelle: `Zero gamma ${prix(d.zero_gamma, dec)}`, rang: 1 },
  ];

  // --- profil de gamma ---
  const series = Object.entries(d.profiles).map(([cle, valeurs], index) => ({
    nom: NOMS_PROFILS[cle] || cle,
    couleur: [COULEURS.serie1, COULEURS.serie2, COULEURS.serie3][index] || COULEURS.serie1,
    valeurs,
  }));
  const echProfil = graphiqueCourbes($("fig-profil"), {
    niveaux: d.levels, series, unite: `Gamma exposure (${echelle(series.flatMap((s) => s.valeurs)).unite} / 1 %)`,
    reperes: reperesCommuns, zoneNegative: d.zero_gamma,
  });
  legende($("legende-profil"), series.map((s) => ({ couleur: s.couleur, nom: s.nom, trait: true })));
  tableau($("tab-profil"), `Profil de gamma en ${echProfil.unite}, par niveau du sous-jacent.`,
    ["Niveau", ...series.map((s) => s.nom)],
    d.levels.map((n, i) => [prix(n, dec), ...series.map((s) => montant(s.valeurs[i], echProfil, 2, true))]));
  $("carte-profil").hidden = false;

  // --- GEX par strike ---
  const ps = d.par_strike;
  const bornes = { xMin: d.from_strike, xMax: d.to_strike };
  // Le tableau est le jumeau lisible du graphique : il porte donc les mêmes
  // strikes. Sur la chaîne entière il en listait 169 quand le tracé en montre 44,
  // et les deux vues ne parlaient plus du même périmètre.
  const tracés = d.strikes
    .map((k, i) => ({ k, i }))
    .filter(({ k }) => k >= d.from_strike && k <= d.to_strike);
  const cadrage = `Strikes tracés uniquement (${tracés.length} sur ${d.strikes.length}), ` +
                  `de ${prix(d.from_strike, dec)} à ${prix(d.to_strike, dec)}.`;
  const echGex = graphiqueBarres($("fig-gex"), {
    strikes: d.strikes, diverge: true, spot: d.spot,
    series: [{ nom: "Gamma exposure net", valeurs: ps.total }],
    unite: `Gamma exposure (${echelle(ps.total).unite} / 1 %)`,
    reperes: [
      ...reperesCommuns,
      { valeur: d.call_wall, ton: "faible", libelle: "Call wall", rang: 2 },
      { valeur: d.put_wall, ton: "faible", libelle: "Put wall", rang: 3 },
    ],
    ...bornes,
  });
  legende($("legende-gex"), [
    { couleur: COULEURS.positif, nom: "Amortit le mouvement (gamma positif)" },
    { couleur: COULEURS.negatif, nom: "Amplifie le mouvement (gamma négatif)" },
  ]);
  tableau($("tab-gex"),
    `Exposition nette par strike, en ${echGex.unite} par mouvement de 1 %. ${cadrage}`,
    ["Strike", "Gamma exposure net", "OI calls", "OI puts"],
    tracés.map(({ k, i }) => [
      prix(k, dec), montant(ps.total[i], echGex, 2, true),
      nf(0, 0).format(ps.call_oi[i] || 0), nf(0, 0).format(ps.put_oi[i] || 0),
    ]));
  $("carte-gex").hidden = false;

  // --- calls vs puts ---
  const echCp = graphiqueBarres($("fig-cp"), {
    strikes: d.strikes, diverge: false, spot: d.spot,
    series: [
      { nom: "Calls", couleur: COULEURS.serie1, valeurs: ps.call },
      { nom: "Puts", couleur: COULEURS.serie2, valeurs: ps.put },
    ],
    unite: `Gamma exposure (${echelle([...ps.call, ...ps.put]).unite} / 1 %)`,
    reperes: reperesCommuns, ...bornes,
  });
  legende($("legende-cp"), [
    { couleur: COULEURS.serie1, nom: "Calls (dealers supposés longs)" },
    { couleur: COULEURS.serie2, nom: "Puts (dealers supposés shorts)" },
  ]);
  tableau($("tab-cp"),
    `Décomposition calls / puts, en ${echCp.unite} par mouvement de 1 %. ${cadrage}`,
    ["Strike", "Calls", "Puts"],
    tracés.map(({ k, i }) => [
      prix(k, dec), montant(ps.call[i], echCp, 2, true), montant(ps.put[i], echCp, 2, true),
    ]));
  $("carte-cp").hidden = false;

  // --- charm et vanna ---
  for (const [cle, colonne, titre, unite] of [
    ["charm", ps.charm, "Charm", "de delta par jour"],
    ["vanna", ps.vanna, "Vanna", "de delta par point de vol"],
  ]) {
    const ech = graphiqueBarres($(`fig-${cle}`), {
      strikes: d.strikes, diverge: true, spot: d.spot,
      series: [{ nom: titre, valeurs: colonne }],
      unite: `${echelle(colonne).unite} ${unite}`,
      reperes: reperesCommuns, hauteur: 300, ...bornes,
    });
    legende($(`legende-${cle}`), [
      { couleur: COULEURS.positif, nom: "Les dealers vendent" },
      { couleur: COULEURS.negatif, nom: "Les dealers achètent" },
    ]);
    tableau($(`tab-${cle}`),
      `${titre} par strike, en ${ech.unite} ${unite}. ${cadrage}`,
      ["Strike", titre],
      tracés.map(({ k, i }) => [prix(k, dec), montant(colonne[i], ech, 2, true)]));
    $(`carte-${cle}`).hidden = false;
  }

  rendreDiagnostics(d, echTotal);
}

function rendreDiagnostics(d, echTotal) {
  const liste = $("liste-diagnostics");
  liste.innerHTML = "";
  const ajouter = (niveau, titre, texte) => {
    const div = document.createElement("div");
    div.className = `diagnostic ${niveau}`;
    const h = document.createElement("h3");
    h.textContent = titre;
    const p = document.createElement("p");
    p.textContent = texte;
    div.append(h, p);
    liste.appendChild(div);
  };

  ajouter("", "Périmètre de ce relevé",
    `${d.ticker} au ${d.date}, échéances ${d.horizon}, gamma ${d.source_gamma === "iv"
      ? "recalculé depuis la volatilité implicite" : "tel que publié par la source"}, ` +
    `temps compté en ${d.time_convention === "heures" ? "heures réelles jusqu'à l'échéance" : "jours de bourse"}. ` +
    `Changer l'un de ces réglages change les niveaux : deux relevés qui n'en partagent pas ne sont pas comparables.`);

  if (d.zero_gamma === null) {
    ajouter("serieux", "Pas de zero gamma dans la plage analysée",
      `Le profil ne change pas de signe entre ${prix(d.from_strike, d.decimals)} et ` +
      `${prix(d.to_strike, d.decimals)}. Élargir la plage, ou allonger l'horizon.`);
  } else if (d.croisements.length > 1) {
    const autres = d.croisements.filter((x) => Math.abs(x - d.zero_gamma) > 1e-9);
    ajouter("attention", "Le profil croise zéro plusieurs fois",
      `${d.croisements.length} croisements au total, également en ` +
      `${autres.map((x) => prix(x, d.decimals)).join(", ")}. Le niveau retenu est le plus ` +
      `proche du spot, mais le régime n'est pas une simple bascule au-dessus / en dessous.`);
  }

  if (d.ecart_gamma && Math.abs(d.ecart_gamma.relatif) > 0.05) {
    ajouter("attention", "Les deux gammas ne sont pas d'accord",
      `Recalculé depuis la volatilité implicite : ${montant(d.ecart_gamma.iv, echTotal, 2, true)}. ` +
      `Publié par la source : ${montant(d.ecart_gamma.publie, echTotal, 2, true)}. ` +
      `Soit ${nf(0, 1).format(d.ecart_gamma.relatif * 100)} % d'écart. ` +
      `Comparer les deux avec le sélecteur « Gamma » avant de conclure.`);
  }

  const parts = Object.entries(d.part_courtes || {});
  const pire = Math.max(0, ...parts.map(([, p]) => p || 0));
  if (pire > 0.2) {
    ajouter("attention", "Les échéances du jour pèsent lourd",
      `Les échéances à 0-1 jour portent ` +
      `${parts.map(([nom, p]) => `${Math.round(p * 100)} % ${nom}`).join(", ")}. ` +
      `Leurs greeks sont instables sur des données différées : près de l'échéance le gamma ` +
      `explose et dépend du prix à la minute. Comparer en excluant ces échéances.`);
  }

  $("diagnostics").hidden = false;
}

// ---=== Chargement ===---

const formulaire = $("filtres");
const message = $("message");
let requeteEnCours = 0;

function parametres() {
  const donnees = new FormData(formulaire);
  const p = new URLSearchParams();
  for (const [cle, valeur] of donnees.entries()) {
    if (valeur !== "") p.set(cle, valeur);
  }
  return p;
}

async function charger() {
  const jeton = ++requeteEnCours;
  const bouton = $("lancer");
  bouton.disabled = true;
  message.className = "message";
  message.textContent = "Chargement du relevé…";
  message.hidden = false;
  // On tient le rendu précédent en retrait plutôt que de vider la page : pas de saut.
  $("contenu").style.opacity = "0.55";

  try {
    const reponse = await fetch(`/api/analyse?${parametres()}`);
    const donnees = await reponse.json();
    if (jeton !== requeteEnCours) return;
    if (!reponse.ok || donnees.erreur) throw new Error(donnees.erreur || `erreur ${reponse.status}`);
    rendre(donnees);
    message.hidden = true;
  } catch (err) {
    if (jeton !== requeteEnCours) return;
    message.className = "message erreur";
    message.textContent = `${err.message}`;
    message.hidden = false;
  } finally {
    if (jeton === requeteEnCours) {
      bouton.disabled = false;
      $("contenu").style.opacity = "1";
    }
  }
}

formulaire.addEventListener("submit", (evenement) => {
  evenement.preventDefault();
  charger();
});
formulaire.querySelectorAll("select").forEach((s) => s.addEventListener("change", charger));

/* Les relevés archivés permettent de rejouer une séance passée sans requête réseau. */
async function chargerSnapshots() {
  try {
    const liste = await (await fetch("/api/snapshots")).json();
    if (!liste.length) return;
    const select = $("replay");
    for (const s of liste.slice().reverse()) {
      const option = document.createElement("option");
      option.value = s.chemin;
      option.textContent = `${s.ticker} — ${s.date.replace("_", " ")}`;
      select.appendChild(option);
    }
    $("champ-replay").hidden = false;
  } catch { /* pas d'archive, pas de sélecteur */ }
}

chargerSnapshots();
charger();
