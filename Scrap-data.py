from selenium import webdriver
from selenium.webdriver.chrome.service import Service
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from webdriver_manager.chrome import ChromeDriverManager

# Fonction pour accéder au shadow root
def expand_shadow_element(element):
    return driver.execute_script('return arguments[0].shadowRoot', element)

# Initialiser le navigateur
driver = webdriver.Chrome(service=Service(ChromeDriverManager().install()))

# Accéder à la page cible
driver.get('https://www.barchart.com/futures/quotes/E6Z24/volatility-greeks/dec-24?futuresOptionsView=merged')

# Attendre que l'élément principal soit présent
try:
    root1 = WebDriverWait(driver, 20).until(
        EC.presence_of_element_located((By.XPATH, '//*[@id="main-content-column"]/div/div[5]'))
    )

    # Récupérer le texte brut de l'élément principal
    text_content = root1.text
    print(text_content)  # Pour voir la structure du texte récupéré

    # Diviser le texte en lignes
    lines = text_content.split('\n')

    # Initialiser les listes pour les calls et puts
    calls = []
    puts = []

    # Itérer sur les lignes et structurer les données
    for i in range(len(lines)):
        if "Call" in lines[i]:  # Identifier les lignes des calls
            try:
                call_data = lines[i:i+10]  # Récupérer les 10 lignes suivantes
                if len(call_data) >= 10:  # Vérifier qu'il y a suffisamment de données
                    calls.append({
                        "Strike": call_data[9],
                        "Type": call_data[0],
                        "Last": call_data[1],
                        "IV": call_data[2],
                        "Delta": call_data[3],
                        "Gamma": call_data[4],
                        "Theta": call_data[5],
                        "Vega": call_data[6],
                        "IV Skew": call_data[7],
                        "Last Trade": call_data[8],
                    })
            except IndexError:
                print(f"Erreur d'accès aux données des calls à la ligne {i}.")
        
        elif "Put" in lines[i]:  # Identifier les lignes des puts
            try:
                put_data = lines[i:i+10]  # Récupérer les 10 lignes suivantes
                if len(put_data) >= 10:  # Vérifier qu'il y a suffisamment de données
                    puts.append({
                        "Strike": put_data[9],
                        "Type": put_data[0],
                        "Last": put_data[1],
                        "IV": put_data[2],
                        "Delta": put_data[3],
                        "Gamma": put_data[4],
                        "Theta": put_data[5],
                        "Vega": put_data[6],
                        "IV Skew": put_data[7],
                        "Last Trade": put_data[8],
                    })
            except IndexError:
                print(f"Erreur d'accès aux données des puts à la ligne {i}.")

    # Afficher les données collectées
    #print("\nCalls:")
    #for call in calls:
        #print(call)

    #print("\nPuts:")
    #for put in puts:
        #print(put)

except Exception as e:
    print(f'Une erreur s\'est produite : {e}')

finally:
    driver.quit()  # Fermer le navigateur à la fin
