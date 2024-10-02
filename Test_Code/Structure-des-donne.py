import pandas as pd

# Lire les données depuis un fichier texte
with open('data-brut.txt', 'r') as file:
    text_content = file.read()

# Diviser le texte en lignes
lines = text_content.split('\n')

# Initialiser les listes pour les calls et puts
calls = []
puts = []

# Itérer sur les lignes et structurer les données
i = 0
while i < len(lines):
    # Ignorer les lignes vides
    if lines[i].strip() == "":
        i += 1
        continue

    # Identifier les lignes des calls ou puts
    if "Call" in lines[i] or "Put" in lines[i]:
        try:
            # Le strike est dans la ligne précédente
            strike = lines[i - 1].strip() if i > 0 else "N/A"
            
            # Récupérer les 9 lignes suivantes (car le strike est déjà récupéré)
            data = lines[i:i+9]
            
            if len(data) >= 9:
                entry = {
                    "Strike": strike,
                    "Type": data[0].strip(),
                    "Last": data[1].strip(),
                    "IV": data[2].strip(),
                    "Delta": data[3].strip(),
                    "Gamma": data[4].strip(),
                    "Theta": data[5].strip(),
                    "Vega": data[6].strip(),
                    "IV Skew": data[7].strip(),
                    "Last Trade": data[8].strip(),
                }
                
                # Ajouter aux calls ou puts en fonction du type
                if "Call" in data[0]:
                    calls.append(entry)
                elif "Put" in data[0]:
                    puts.append(entry)
                    
            # Avancer l'index de 9 (en plus du call/put lui-même)
            i += 9
        except IndexError:
            print(f"Erreur d'accès aux données à la ligne {i}.")
    else:
        i += 1  # Passer à la ligne suivante

# Afficher les données collectées
print("\nCalls:")
for call in calls:
    print(call)

print("\nPuts:")
for put in puts:
    print(put)
