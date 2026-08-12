"""Rend les modules du projet importables depuis tests/, sans fenêtre matplotlib."""

import sys
from pathlib import Path

import matplotlib

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

# Backend non interactif : les tests écrivent des PNG, ils n'ouvrent rien.
matplotlib.use("Agg")
