# Chemin du fichier d'entrée et de sortie
input_file_path = 'options_data.txt'
output_file_path = 'options_data.csv'

# Fonction pour extraire les données et les structurer
def process_line(line):
    # Supprimer les accolades et diviser les paires clé-valeur
    line = line.strip()[1:-1]
    data = {}
    for pair in line.split(", "):
        key, value = pair.split(": ")
        data[key.strip("'")] = value.strip("'")
    return data

# Lire le fichier d'entrée et écrire dans le fichier de sortie CSV
with open(input_file_path, 'r') as infile, open(output_file_path, 'w') as outfile:
    reader = infile.readlines()
    
    # Ecrire les en-têtes manuellement dans le fichier de sortie
    outfile.write("StrikePrice,Type,Last,IV,Delta,Gamma,Theta,Vega,IV Skew,Last Trade\n")
    
    # Parcourir chaque ligne et traiter uniquement les lignes avec des données d'options
    for line in reader:
        if line.startswith("{'Strike'"):
            data = process_line(line)
            # Créer une ligne CSV à partir des valeurs extraites
            csv_line = f"{data['Strike']},{data['Type']},{data['Last']},{data['IV']},{data['Delta']},{data['Gamma']},{data['Theta']},{data['Vega']},{data['IV Skew']},{data['Last Trade']}\n"
            outfile.write(csv_line)

print(f"Le fichier CSV est créé à {output_file_path}")
