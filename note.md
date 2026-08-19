De ma comprehension generale, l'avantage ici du DAS NVMe assemble repose sur le fait que le stockage est du SSD NVMe et donc sans disque. On peut avoir cet avantage avec du SSD classique sans que ce soit du NVMe, qui est plus rapide et plus cher.

Un interet du NAS dans le systeme actuel, c'est qu'il va gerer l'enregistrement lui meme avec Synology Surveillance Station. Si on enleve le NAS, on doit gerer l'enregistrement nous meme. Ca veut dire que mon logiciel doit gerer ca. C'est pas tres complique, mais c'est un peu plus fragile et ca veut dire que l'ordinateur devient un composant critique comme etait le NAS et que s'il a un probleme (il s'eteint), l'enregistrement s'arrete (pas perdu mais s'arrete).

Version actuelle :
- NAS s'occupe de l'enregistrement et est moins sujet a des coupures
- Le PC est independant de l'enregistreemnt et peut s'arreter sans que l'enregistrement s'arrete

Version DAS :
- Le PC doit gerer l'enregistrement
  - Un peu de travail pour gerer tout les petits chiants d'interruption
  - On gere le format, donc meme si le PC s'arrete, on peut ne rien perdre.
- L'onduleur devient moins critique, il n'alimenterait que le DAS pendant maximum quelques minutes.
  - Si le PC portable alimente le DAS, plus besoin d'onduleur du tout
- Le PC doit etre legerement plus puissant mais rien de fou non plus, tes recommendations etaient deja overkill
- Le NAS n'est plus si critique, on peut probablement reduire un peu les criteres, le debit n'est plus important.

Les deux options ont des avantages et inconvenient, que ce soit a ton niveau, ou au niveau du code. 

8 To me semblent largement suffisant mais je prefere un peu RAID 5 de 4x4To ou meme 4x2To. Avec un RAID 1 a 2 disks tu perds 50% de stockage, avec un RAID 5 a 4 disks, tu perds 25% de stockage. A voir les options de produit mais je ne penses pas qu'on doive prendre du NVMe pour le DAS, du SSD classique peut suffire a mon avis.


--- 

Confirm go with DAS :
- Actual DAS or two SSD disk with software layer ? 


# Tour 
La tour me semble clairement overkill, on peut concentrer la config autour de la carte graphique et reduire le reste :
- RAM : 96 Go de ram sont beaucoup trop, 32 Go sont suffisant. 
- SSD : Le NAS sera la principale source de stockage, donc 512 Go devraient etre suffisant, on peut prendre 1To pour etre a l'aise. Pas besoin de deux disques
- Processeur : il ne fera pas les calculs en tant que tel, on peut reduire
- L'alimentation : il faut en general pas hesiter la dessus, mais 1300 est enorme pour une carte graphique qui consomme 300

# NAS

- Le NAS Synology DS925+ devient overkill si on a plus besoin d'un gros debit. Le DS425+ est largement suffisant, meme moins
- Je ne pense pas qu'on ait besoin de autant de stockage a moins de garder 1 mois de rush. 3/4 disques de 4To me semblent bien suffisant. A toi de voir en fonction des differences de prix et si tu penses qu'on peut record pendant plusieurs semaines. Aussi a reflechir si on transfert au bureau tout les jours ou pas.

# Operateur en live

- Qu'entends tu par "balance les prises de vue"
- J'avais prevu que l'operatuer active uniquement les cameras utilisee en live pour reduire la post prod au max. 
- On peut reflechir a analyse IA qui desactive des cameras en live mais alors il faut de la connexion

# PC Portable

- Si on a beaucoup de stockage sur les SSD de transfert on peut uplaod dessus au fur et a mesure de la journee, donc pas besoin de 4To sur le laptop.
- Je vois aussi que le SSD n'est pas disponible sur le site de Framework, c'est tout a fait possible de l'acheter autre part, mais c'est encore plus cher que ce que le site de Framework propose, une raison ? 
- Ajoutons un port SD/microSD pour la carte SD/microSD des camera
- Est ce qu'on a vraiment besoin d'un ecran externe si on a le portable? 
- On peut prendre un cadre de couleur si tu te sens d'humeur joviale

# Stockage

- Les cameras n'enregistreront sur leur cartes SD qu'en cas de probleme (probleme de connexion avec le laptop). Si tu veux etre vraiment safe on peut prendre une journee, mais je doute meme qu'on puisse continuer la journee s'il y a un probleme pareil qu'on ne sait pas resoudre.
