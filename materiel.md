Oui. En prenant les mails du plus récent au plus ancien, puis en appliquant tes décisions d'aujourd'hui, l'architecture matérielle actuelle devient:

```text
Caméras + microSD
        |
Switch PoE sur onduleur
        |
Laptop de régie
        |
SSD externe A + SSD externe B, écriture simultanée
        |
Transfert contrôlé vers le NAS du bureau
```

Il n'y a plus de NAS ni de DAS dans le container.

## 2. Cartes microSD

- Modèle envisagé: AXIS Surveillance Card 512 Go, référence `02365-001`.
- Une carte par caméra.

## 3. Illuminateurs infrarouges

- 4 Raytec `VAR2-POE-i2-1`, 850 nm.
- Alimentés en PoE.
- 8 W par illuminateur.

## 4. Switch PoE

**Options discutées**

[ ] Zyxel GS1920-24HPv2: 24 ports, budget PoE élevé.
[ ] TP-Link SG3428XMP: 24 ports, PoE élevé, SFP+ 10 Gb/s.
[x] TP-Link Omada SG2218P V2/V2.20: 16 ports, 150 W PoE, silencieux et sans contrôleur obligatoire.

> verifie que c'est bien silencieux et les chiffres matchent

- Pour 5 caméras, 4 Raytec et le laptop, 16 ports sont suffisants.

## 5. Onduleur

**Options discutées**

- APC SMT1500IC: abandonné car trop lourd et surdimensionné.
- APC SMT750IC: 750 VA / 500 W.
- Eaton 5SC750I: 750 VA / 525 W, option préférée dans les anciens échanges.
- Une phase intermédiaire avait supprimé complètement l'onduleur.

> ajoute des infos et compare.
> on peut redimensionner comme pas de nas

- Sur le switch PoE
- Les SSD externes portables seront normalement alimentés par le laptop.


## 6. Laptop de régie

- Framework 13
- Al 5 340 CPU
- 2.2k display
- 1x32gb ram
- 1T stockage
- No OS
- Transparent Black or Green bezel. Something funny
- Belgian keyboard for azerty, or Internation english for qwerty
- Power adapter 60w
- Expansion card
  - 3 usb-c
  - 1 usb-a
  - 1 ethernet
  - 1 microSD

> 2x16 or 1x32 ram ? if the the different is very small, 1x32 is cheaper

## 7. SSD externes

- 2 x 4To
- usb-c
- with cable
- No specific, I trust SanDisk or Samsung

## 8. Ecran, dock, clavier et souris

- Ecran : je te laisse regarder en fonction de comment tu veux l'installer. Mon seul critere c'est qu'il ait une fiche usb-c pour l'image et charger le laptop en meme temps avec suffisemment de puissance (le chargeur du laptop est 60W).

## 9. NAS du bureau

> make some recommendation, I think DS425+ is good enough but tell more
> I'd like a 4 bay NAS

- 3x4To, can add a 4To if needed later

## 10. Station IA du bureau

- Est ce qu'il faut choisir maintenant ? C'est un gros choix et je suis pas encore convaincu de comment faire ca
- Mon questionnement principale est sur le GPU. Tout le reste est overkill

## 11. Audio

> is the focusrite really necessary ? what cost for what benefit ?


- 2 Audio-Technica PRO45.
- 1 Focusrite Scarlett 2i2 4th Gen.
