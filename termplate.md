# Termplate

## template-pohjainen terminaali-UI kuvauskieli

Oct 27, 2026 · @Seppo Säämäki

## Speksin runko

Luonnos 28.10.2026 (päivitetty CLARIFICATIONS.md:n vastausten mukaan; lisätty yhden tiedoston sovellus ja upotetut skriptit; peliohjain ja teematiedostot poistettu, `ansi keep` ja komennon `- timeout` tarkennettu). Nimi Termplate. Pohjana RLML 0.3:n käyttöliittymädata (dataformaatit, luvut 9.3, 10 ja 11) ja moottorin vastuut (luvut 7.4, 7.5, 7.7 ja 8). Luvut ovat runkoja: jokaisessa on sisältö, RLML:stä perityt ratkaisut ja avoimet kohdat.

### 0. Tarkoitus ja rajaus

- Termplate kuvaa tekstikäyttöliittymän datana: asettelun, paneelit, valikot, syötteet, näppäimet ja tekstit. Logiikka on isännällä (bash-skripti, muu ohjelma tai RLML-moottori).
- **Kohde:** bash-skriptien käyttöliittymät, ylläpitotyökalut, asennusohjelmat, pienet dashboardit.
- **Ei tavoitteena:** yleiskäyttöinen widget-kirjasto, käyttöliittymät, jotka vaativat hiiren (hiiri on lisäsyöte, luku 9.1), grafiikka.

* **Suhde RLML:ään:** RLML-moottori on yksi Termplaten isäntä. Termplaten syntaksi on RLML:n UI-osan osajoukko ja laajennus, ja sama tiedosto kelpaa molemmille, kun lausekkeet ovat isännän kieltä.

### 1. Periaatteet

- **Piirros on totuus.** Asettelu ja paneelit piirretään ASCII:na, ja legendi antaa merkityksen. Koodissa ei ole koordinaatteja.
- **Data, ei ohjelma.** Termplate ei sisällä silmukoita eikä muuttujien laskentaa, vain sidontoja isännän arvoihin. Upotettu skripti (luku 6.1) on isännän koodia, jota Termplate kuljettaa mutta ei tulkitse.
- **Virheet latauksessa.** Tuntematon avain, kenttä, valikko, näppäin tai kehä on latausvirhe, ei ajonaikainen yllätys.
- **Tekstit avaimina.** Piirrokseen ja valikkoihin ei kirjoiteta käännettäviä sanoja. Kiinteä teksti piirroksessa on `text`-kenttä tai nimetty kenttä (`[name___]`), ei sanoja kuvana.
- **Käyttäjä voi paikata.** Näppäimet ja asetukset ovat käyttäjän muokattavissa ilman, että ohjelman tiedostoihin kosketaan.
- **Isäntä on vaihdettava.** Lausekkeet arvioi isäntä rajapinnan kautta (luku 16).

### 2. Tiedostomuoto

- Perii RLML:n perussyntaksin: lohkon otsikko, ` -  `-rivit, `legend:`-alilohko, `>`-kommentit, `include` jokereineen, `namespace`, `patch` / `override` / `new` / `drop` (RLML D 2, 4.8).
- Tiedostopääte `.termplate` tai `.rlml`. Juuritiedosto aloittaa `app NIMI` -lohkolla (vastine RLML:n `game`-lohkolle).

```
app BACKUPTOOL
- termplate 0.1
- layout MAIN
- language fi

include ui/*
include text/*
```

- **Versio:** juuritiedoston `app`-lohkossa on pakollinen kenttä `- termplate VERSIO`. Lataus hylkää tiedoston, jonka versiota toteutus ei tue, ja puuttuva kenttä on latausvirhe.
- **Widget-tiedostot:** widget-tilassa (luku 14.1) `app`-lohko on valinnainen. Ilman sitä sovelluksen nimi on tiedostonimi isoin kirjaimin (`ui.termplate` → `UI`), versioksi oletetaan uusin tuettu, ja `termplate check` varoittaa puuttuvasta versiosta. Sovellus- ja koprosessitila vaativat `app`-lohkon.
- **Yhden tiedoston sovellus:** `include` on valinnainen. Koko sovellus voi olla yhdessä tiedostossa: kaikki lohkot, tekstit (oletustekstit ja `text LANG` -lohkot, luku 10), tyyli (luku 11) ja skriptit (`run:`-alilohkot, luku 6.1). Sovellustiedoston ulkopuolelle jäävät vain käyttäjän ohitukset ja asetukset (luku 13), koska ne kuuluvat käyttäjälle.

### 3. Arvot, muuttujat ja lausekkeet

RLML:ssä lausekkeet ovat RogueScriptiä. Termplatessa lausekkeen muoto riippuu isännästä; tämä luku kuvaa shell-isännän muodon.

- **Muuttujat** ilmoitetaan `vars`-lohkossa tyypin ja oletusarvon kanssa. Tyypit: `text`, `number`, `bool`, `list`, `choice A B C`.

```
vars
- TARGET text "/mnt/backup"
- VERBOSE bool false
- LEVEL number 3 range 1..9
```

- **Lausekemuodot:**
  - `$NIMI` – muuttujan arvo.
  - `$(komento)` – komennon stdout, rivinvaihto lopusta poistettuna.
  - `ok $(komento)` – tosi, kun komennon paluuarvo on 0 (ehdoissa).
  - Literaalit ja vertailut: `$LEVEL > 5`, `$TARGET == ""`, `not`, `and`, `or`.
- **Ehdot** (`hide when`, `disabled when`, `when`) ovat bool-lausekkeita.
- **Lista** on rivinvaihdoin erotettu stdout tai `list`-muuttuja.
- **Ei aritmetiikkaa:** lausekkeissa sallitaan vertailut ja `not`, `and`, `or`, mutta ei laskutoimituksia. Laskenta tehdään skripteissä, erillisissä tai upotetuissa (luku 6.1).
- Lausekkeet ovat yksirivisiä. Monirivinen laskenta kirjoitetaan `command`-lohkoon, joka tallentaa tuloksensa muuttujaan (`- into`).

### 4. Asettelu

Kuten RLML D 11.1.

```
layout MAIN
sssmm
sssmm
lllll

legend:
- s STATUS size.width 30
- m MAIN_MENU
- l LOG size.height 6
```

- Alueiden on oltava suorakulmioita, ja koko asettelun on oltava jaettavissa suorilla viivoilla.
- Puuttuva mitta täyttää jäljelle jäävän tilan. Piirros on pienin sallittu koko.
- Alueeseen sijoitetaan paneeli, valikko (uusi: RLML:ssä valikot ovat vain modaaleja), loki tai tulosteen näkymä.
- **Uutta:** `- m MAIN_MENU` upottaa valikon pysyvästi alueeseen; fokus siirtyy alueiden välillä toiminnoilla `next_area` ja `prev_area`.
- **Numeroidut alueet:** fokusoitavat alueet (valikko, loki, syöte, syötekentällinen paneeli) numeroidaan 1–9 siinä järjestyksessä, jossa ne ovat legendissä. Toiminto `focus N` siirtää fokuksen alueeseen N; modaalin ollessa auki se ei tee mitään. Näppäinrivi: `- focus 2 g`.
  - Oletusnäppäimet `ALWAYS`-kontekstissa ovat `alt+N` ja `fN`. Sovelluksen oma `focus N` -rivi korvaa ne, eikä oletusta lisätä näppäimelle, jonka sovellus käyttää jossain kontekstissa.
  - macOS:n Terminalissa Option-näppäin ei oletuksena lähetä `alt`-yhdistelmiä, joten `fN` on varmempi.
  - Pelkät numerot jäävät syötteille.
  - Asetus `settings - area_numbers on` näyttää numeron reunallisen alueen yläreunassa.
- **Kapea pääte:** jos pääte on asettelun piirrosta pienempi, käytetään vaihtoehtoista asettelua, jos sellainen on määritelty (`layout MAIN.narrow when width < 80`). Muuten näytetään varoitus.

### 5. Paneelit

Kuten RLML D 11.5 ja M 7.7.

```
panel STATUS refresh 2s
TTTTTTTTTTTTTTTTTTTTTTTTTT
[disktxt_] dddddddddddddd
[loadtxt_] llllllllllllll
pppppppppppppppppppppppppp

legend:
- T text STATUS.TITLE center
- disktxt text STATUS.DISK
- loadtxt text STATUS.LOAD
- d value $(df -h / | awk 'NR==2 {print $4}') right
- l value $(uptime | awk -F'load averages?: ' '{print $2}')
- p bar $USED of 100
```

- Saman merkin vaakajakso on yksirivinen kenttä ja suorakulmio monirivinen. Legendissä mainitsemattomat merkit ovat kuvaa.
- **Ei sanoja piirroksessa:** nimiöt ovat `text`-kenttiä (`- disktxt text STATUS.DISK`). Näin piirroksen kirjaimet eivät sekoitu kenttämerkkeihin (RLML D 11.5) ja nimiöt käännetään kuten muutkin tekstit.
- **Ei reunoja piirroksessa:** reunat ja otsikkorivi tulevat tyylistä (luku 11). Piirros kuvaa vain sisällön. Lataus varoittaa paneelista, jonka piirroksen ensimmäinen ja viimeinen rivi koostuvat reunamerkeistä.
- **Merkkien leveys:** leveydet lasketaan näyttösarakkeina (Unicode-leveys), ei merkkeinä. Leveä merkki (CJK, emoji) piirroksessa on latausvaroitus kuten RLML:ssä (M 2.5); kenttien arvoissa se on sallittu ja leikataan sarakkeiden mukaan.
- Kenttälajit: `text AVAIN`, `value LAUSEKE`, `param NIMI` (RLML:stä).
- **Uudet kenttälajit:** `bar ARVO of MAKSIMI` (edistymispalkki), `input MUUTTUJA` (muokattava kenttä, luku 8), `spinner when EHTO`.
- **Päivitys:** RLML laskee arvot joka välikehyksessä; shell-isännässä komennot ovat kalliita, joten paneeli saa `refresh AIKA`, ja ilman sitä arvot lasketaan avattaessa ja muuttujan muuttuessa.
- Komennot ajetaan taustalla aikakatkaisulla (luku 15). Kenttä näyttää edellisen arvon, kunnes uusi valmistuu, eikä uutta päivitystä aloiteta, ennen kuin edellinen on valmis.
- Modaalinen paneeli: `open PANEELI(…)`, suljetaan toiminnoilla `confirm` tai `back`.
- **Pidemmät tunnisteet:** yhden merkin kenttien lisäksi kentän voi merkitä nimellä hakasulkeissa Informix-4GL:n tapaan (`[disk____]`). Hakasulkeet ja täyte kuuluvat kentän leveyteen, ja legendi viittaa kenttään nimellä (`- disk value …`).

### 6. Komennot ja toiminnot

RLML:ssä iso nimi on komento ja pieni moottorin toiminto. Termplatessa komento on `command`-lohko.

```
command BACKUP
- confirm BACKUP.CONFIRM
- run ./backup.sh
- env TARGET=$TARGET VERBOSE=$VERBOSE
- output LOG
- then open DONE
- on_error open FAILED
```

- `run` ajaa komennon. Parametrit välitetään **aina ympäristömuuttujina**, ei merkkijonoon sijoittamalla (luku 15).
- `- shell TULKKI` valitsee tulkin, jolla `run` ajetaan: `sh` (oletus), `bash`, `zsh` tai polku tulkkiin. Yksirivinen `run` ajetaan muodossa `TULKKI -c RIVI`. Monirivinen skripti kirjoitetaan `run:`-alilohkoon (luku 6.1).
- **Ympäristö:** komento perii Termplaten oman ympäristön, ja siihen lisätään vain `env`-rivien muuttujat, listavalikon valinta `IT` (luku 7.1) ja `- secret NIMI env` -rivillä pyydetyt salaiset arvot. Muut `vars`-muuttujat ja parametrit eivät näy komennolle, ellei niitä nimetä `env`-rivillä.
- **Lausekkeiden ympäristö:** lausekkeiden `$( … )`-korvaukset (luku 3) saavat tarkoituksella kaikki muuttujat, parametrit, `$it`:n ja `$VALUE`:n, jotta `ok $(test -d "$TARGET")` toimii ilman `env`-rivejä. Nekin saavat arvot ympäristömuuttujina, eivät komentoriviin sijoitettuina. Salaisia arvoja lausekkeet eivät saa.
- `run KOMENTO interactive` antaa koko päätteen ohjelmalle (`$EDITOR`, `less`, `ssh`): Termplate keskeyttää piirron, ja käyttöliittymä palaa, kun ohjelma päättyy.
- `- window [tab|split|popup]` ajaa komennon erillisessä ikkunassa odottamatta sitä (luku 15.1).
- `output ALUE` ohjaa stdoutin ja stderrin loki-alueeseen; ilman sitä tuloste näkyy modaalina, joka suljetaan kuittauksella.
- `confirm AVAIN` kysyy vahvistuksen ennen ajoa. Tekstissä `{IT}` on valittu rivi tai rivit.
- `then` ja `on_error` valitsevat jatkon paluuarvon mukaan.
- **Sisäiset toiminnot** (pienellä): `quit`, `back`, `close`, `open`, `set MUUTTUJA ARVO`, `toggle MUUTTUJA`, `refresh`, `next_area`, `prev_area`, `focus N`, `stop KOMENTO` (keskeyttää käynnissä olevan komennon, luku 15), `emit TAPAHTUMA` (koprosessille, luku 14).
- **Paluuarvo ja ketjut:** `- into MUUTTUJA` tallentaa komennon stdoutin muuttujaan. Komento voi ajaa toisen komennon (`then KOMENTO`), kuten RLML:n komentoketjut.

#### 6.1 Upotetut skriptit

Monirivinen skripti kirjoitetaan komennon `run:`-alilohkoon, jolloin koko sovellus mahtuu yhteen tiedostoon (luku 2).

````
command BACKUP
- shell bash
- env TARGET=$TARGET
- output LOG
- then open DONE
run:
```bash
set -euo pipefail
mkdir -p "$TARGET"
rsync -a --delete ~/docs/ "$TARGET/"
echo "Valmis: $(du -sh "$TARGET" | cut -f1)"
```
````

- **Muoto:** `run:` on alilohko kuten `legend:`, ja sen sisältö on aina markdown-koodiaidassa. Aita on pakollinen, koska skriptin `#`-kommentit ja `>`-uudelleenohjaukset näyttäisivät muuten Termplaten otsikoilta ja kommenteilta. Aidan sisältö annetaan tulkille sellaisenaan, myös tyhjät rivit ja sisennys.
- **Tulkki:** `- shell` valitsee tulkin. Ilman sitä aidan kielitunniste `sh`, `bash` tai `zsh` valitsee tulkin, ja ilman kumpaakaan tulkki on `sh`. Jos `shell` ja kielitunniste ovat ristiriidassa, se on latausvirhe.
- **Ajo:** skripti kirjoitetaan väliaikaistiedostoon (oikeudet 600) ja ajetaan muodossa `TULKKI TIEDOSTO`; tiedosto poistetaan, kun komento päättyy. Näin skripti ei näy `ps`-listauksessa, eikä se vie stdiniä, joka jää `- secret NIMI stdin` -arvolle ja `interactive`-ohjelmille.
- **Kuten `run`:** ympäristö (vain `env`, `IT` ja pyydetyt salaisuudet), työhakemisto, aikakatkaisu, taustasuoritus, `back` ja `stop`, `output`, `into`, `then`, `on_error`, `interactive` ja `window` toimivat samoin kuin yksirivisellä `run`-rivillä. `window`-ikkunassa kääre poistaa skriptitiedoston komennon jälkeen.
- **Lataustarkistukset:**
  - `- run` ja `run:` samassa komennossa on latausvirhe.
  - Puuttuva tulkki on latausvirhe.
  - Skriptin syntaksi tarkistetaan (`TULKKI -n`), kun tulkki tukee sitä (`sh`, `bash`, `zsh`). Virhe on latausvirhe, ja sen rivinumero muunnetaan Termplate-tiedoston riviksi.
- **Ei lausekkeissa:** lausekkeet (`value`, `when`, `source`) pysyvät yksirivisinä. Pidempi laskenta tehdään komennossa, joka tallentaa tuloksen muuttujaan (`- into`), ja kenttä näyttää muuttujan.
- **Paikkaus:** `patch command NIMI` voi antaa uuden `run:`-alilohkon, joka korvaa vanhan kokonaan. Skriptin yksittäisiä rivejä ei paikata.

### 7. Valikot

Kuten RLML D 11.2 ja M 7.5.

```
menu MAIN_MENU
- BACKUP
- RESTORE disabled when not ok $(test -d $TARGET)
- SETTINGS open SETTINGS
- QUIT quit

menu SETTINGS context POPUP
- VERBOSE toggle VERBOSE
- LEVEL adjust LEVEL 1..9
- BACK back

menu FAILED context POPUP final
- RETRY BACKUP
- QUIT quit
```

- Kohta ilman toimintoa ajaa samannimisen komennon tai sisäisen toiminnon.
- `hide when`, `disabled when`, `adjust`, pinoutuminen, `size` ja `context` kuten RLML:ssä.
- `final` on valikon määre (ei kohdan): `back` ja `close` eivät sulje valikkoa, ja lataus tarkistaa, että valikosta pääsee pois jollakin kohdalla.
- Tekstit: `<VALIKKO>.TITLE`, `<VALIKKO>.TEXT`, `<VALIKKO>.<KOHTA>`.
- Ehdot arvioidaan, kun valikko avautuu, muuttuja muuttuu, komento päättyy tai ajetaan `refresh` (komentoja sisältävät taustalla, luku 15). Tiedostojärjestelmästä riippuvat ehdot (`ok $(test -d …)`) päivittyvät siis `refresh`-toiminnolla tai paneelin päivitysvälillä, eivät ajastimella.

#### 7.1 Listavalikot

```
menu LOGS list
- source $(ls /var/log/*.log)
- row FILE: $it
- sort $it desc
- filter
- then run "less -- \"$IT\""
- keep
```

- `source`, `row`, `sort`, `hide when`, `disabled when`, `then`, `keep` ja `<VALIKKO>.EMPTY` kuten RLML:ssä.
- Valittu rivi on `$it` lausekkeissa ja `IT` komennon ympäristössä.
- **Näppäimellä ajettu komento** saa `IT`:ksi aktiivisen listavalikon merkityt rivit (rivinvaihdoin erotettuna) tai, jos merkittyjä ei ole, kursorin rivin. Näin esimerkiksi `- COPY f5` toimii valitulle tiedostolle.
- `select` (oletuksena `insert`) merkitsee rivin ja siirtää kursorin seuraavalle, kuten Midnight Commanderissa ja fzf:ssä.
- Valinnan jälkeen `keep`-listan haku (`filter`) tyhjenee: haku löysi rivinsä.
- Rivit saavat kirjaimet a–z näkyvässä järjestyksessä (RLML:n kokoelmakirjaimia ei ole).
- **Uutta:** `filter` lisää kirjoittamalla suodattavan haun (fzf-tyyli); `multi` sallii monivalinnan, jolloin `IT` on rivinvaihdoin erotettu lista.
- **Sarakkeet:** `columns` määrittää rivin sarakkeet nimineen, leveyksineen (`*` täyttää) ja tasauksineen (`- columns NAME:* SIZE:8 right`). `row` antaa arvon kullekin sarakkeelle nimellä, ja sarakeotsikot tulevat avaimista `<VALIKKO>.<SARAKE>`.

### 8. Syötteet

RLML:n `ask`-lähde ja luku-/tekstisyöte yleistettynä.

```
input TARGET_DIR
- var TARGET
- kind path
- validate ok $(test -d "$VALUE")
- error TARGET_DIR.INVALID
```

- Lajit: `text`, `number`, `password` (ei kaiuta, ei lokia), `path` (täydennys), `choice`.
- Avataan valikosta (`open TARGET_DIR`), upotetaan paneeliin (`input`-kenttä) tai sijoitetaan asettelun alueeseen.
- Asettelun alueessa syötettä muokataan paikallaan, kun alueella on fokus, ja muuttujan uusi arvo (`set`, koprosessin `set`) näkyy kentässä heti.
- `- then TOIMINTO` ajetaan, kun kelvollinen arvo on tallennettu (esim. `- then ASK`).
- `- clear` tyhjentää kentän `then`-toiminnon jälkeen, kuten keskustelun tai komentorivin syöte. `then`-komento on jo saanut arvon ympäristöönsä.
- **Polun täydennys:** `path`-syötteessä `tab` täydentää (syötekonteksti on modaalinen, joten se ei ole `next_area`).
  - Ensimmäinen `tab` täydentää yhteisen alkuosan.
  - Toinen `tab` avaa osumista suodatettavan listavalikon (`TERMPLATE.COMPLETE`); hakemistot päättyvät `/`-merkkiin. Valinta täyttää kentän, ja `back` sulkee listan.
  - Suhteellinen polku luetaan työhakemistosta, `~/` kotihakemistosta. Piilotiedostot näytetään vain, kun kirjoitettu nimi alkaa pisteellä.
  - Yli 200 osumaa: listassa näytetään 200 ensimmäistä ja `…`.
#### 8.1 Lomakkeet

Paneeli, jossa on `input`-kenttiä, on lomake: kenttiä muokataan paikallaan ja ne vahvistetaan yhdellä kertaa.

```
panel NEW_USER then CREATE_USER
nnnnnn [user______]
aaaaaa [age_]

legend:
- n text NEW_USER.NAME
- a text NEW_USER.AGE
- user input USER_IN
- age input AGE
```

- **Siirtyminen:** `tab` ja `down` seuraavaan kenttään, `shift+tab` ja `up` edelliseen. `path`-kentässä `tab` täydentää, ja `choice`-kentässä nuolet vaihtavat arvoa. Modaalissa siirto kiertää; asettelun alueessa viimeisen kentän jälkeen `tab` siirtyy seuraavaan alueeseen.
- **Vahvistus** (`confirm`, oletuksena `enter`): kaikki kentät jäsennetään muuttujansa tyypillä ja tarkistetaan `validate`-ehdollaan. Ensimmäinen virhe näytetään, kursori siirtyy sen kenttään, eikä mitään tallenneta. Kun kaikki kelpaavat, muuttujat asetetaan, modaali sulkeutuu ja otsikon `then TOIMINTO` ajetaan. `then` on otsikon viimeinen määre.
- **Peruutus** (`back`): muokkaukset hylätään ja modaali sulkeutuu; asettelun alueessa kentät palaavat muuttujien arvoihin.
- Hiiren klikkaus kenttään valitsee sen; klikkaus lomakkeeseen ei vahvista sitä.
- Lomake käyttää `INPUT`-kontekstin näppäimiä ja tyyliä. Modaalinen paneeli saa aina reunan tyylistä (luku 11) ja otsikon tekstistä `<PANEELI>.TITLE`; lomakkeen alarivillä on vihje `TERMPLATE.FORM_HINT` tai virheilmoitus.
- Lataus hylkää `then`-määreen paneelilta, jossa ei ole syötekenttiä.

### 9. Näppäimet, hiiri ja kontekstit

Kuten RLML D 11.3 ja M 8.

```
keys ALWAYS
- quit ctrl+q

keys MENU
- confirm enter space
- back esc
- cursor.north k up
- cursor.south j down

keys MAIN
- BACKUP b
```

- Kontekstit pinoutuvat; `ALWAYS` on aina voimassa, `MENU` valikoissa ja modaaleissa.
- Näppäin voi sitoa komennon suoraan (`- BACKUP b`).
- **Kursoritoiminnot** nimetään kuten RLML:ssä: `cursor.north`, `cursor.south`, `cursor.west`, `cursor.east`. Lyhyet nimet `up`, `down`, `left`, `right` hyväksytään synonyymeinä, mutta `check` ja tekstien `{^…}` käyttävät RLML-nimiä.
- Tekstien `{^confirm}`-paikkamerkki näyttää nykyisen sidonnan (luku 10).
- **Ei peliohjainta:** RLML:n `pad`-osiota ei tueta. Termplate-tiedostossa `pad`-lohko on latausvirhe. Steam Deckin kaltaisilla laitteilla Steam Input voi muuttaa ohjaimen näppäimistö- ja hiirisyötteeksi.

#### 9.1 Hiiri

Hiiri on lisäsyöte: kaiken voi tehdä myös näppäimistöllä. Sama koskee shell- ja RLML-isäntää.

- Klikkaus valitsee valikon kohdan tai listarivin ja siirtää fokuksen alueeseen.
- Rulla vierittää listaa, lokia ja monirivistä kenttää.
- Klikkaus `input`-kenttään aloittaa muokkauksen.
- Hiiritapahtumat (`click`, `wheel`) sidotaan `keys`-lohkossa kuten näppäimet (`- confirm enter click`), joten käyttäjä voi ohittaa ne.
- **RLML-isännässä:** osoittimen alla olevan entiteetin kuvaus näkyy statusrivillä (hover). Kartan ruudun klikkaus kulkee sinne A\*-haulla, ja moottori muuttaa reitin kävelykomennoiksi. Termplate antaa klikatun ruudun myös `ask`-lähteelle.

### 10. Tekstit ja kielet

Kuten RLML D 10.

- Kielitiedosto: `AVAIN teksti`. `{NIMI}`-paikkamerkit, `{@AVAIN}`-viittaukset, `{^TOIMINTO}`-näppäimet.
- Puuttuva käännös näkyy avaimena.
- Kieli valitaan asetuksella tai ympäristöstä (`LANG`), ja oletuskieli voidaan nimetä `app`-lohkossa.
- **Oletustekstit:** kohdalle ja kentälle saa antaa oletustekstin suoraan (`- BACKUP "Varmuuskopioi"`), ja sitä käytetään, kun kielitiedostossa ei ole avainta. Kielitiedoston voi myös upottaa päätiedostoon (`text fi` -lohko), ja useampi kieli saadaan useammalla `text LANG` -lohkolla. Näin yhden tiedoston sovellus (luku 2) voi olla myös monikielinen.

### 11. Tyyli

Kuten RLML D 9.1 ja 9.3.

- `colors`-lohko: paletti (`- STONE 5a5a6e`). Termplate hyväksyy lisäksi päätteen omat värit pienellä (`red`, `dark_grey`, `default`), jotka noudattavat käyttäjän päätteen teemaa.
- `panels`-osio (reunat, värit, himmennys, valittu rivi). Roolien muunnelmat: `MENU.selected`, `MENU.disabled`, `LOG.stderr`, `.title` jne. Reunat: `single`, `double`, `rounded`, `heavy`, `ascii`, `none`.
- **Reunat ovat vain tyylissä** (kuten RLML:ssä); piirroksissa ei ole reunoja. Modaalinen laatikko saa aina reunan (tyylistä tai `single`), asettelun alue vain `panels`-rivistä.
- `NO_COLOR` kunnioitetaan.
- **Ei teematiedostoja:** teema on käyttäjän ohitus (luku 13), joka paikkaa sovelluksen `colors`- ja `panels`-osioita. Olemassa oleva rivi korvautuu, uusi merkitään `new`, ja sovelluksen oma tyyli on aina pohja. Teemaa ei valita asetuksella. Sovelluksen kannattaa käyttää päätteen omia värejä, jolloin käyttäjän pääteteema riittää.

### 12. Loki ja tulosteet

Kuten RLML D 11.4 ja M 7.4.

```
log LOG history 500 more
```

- Komentojen tuloste ja `log`-viestit kirjoitetaan loki-alueeseen.
- `more` sivuttaa (`--More--`), `history N` säilyttää rivit vieritettäväksi.
- stderr erotetaan tyylillä.
- ANSI-värikoodit joko säilytetään tai poistetaan: `log NIMI ansi keep|strip`, oletus `strip`.
  - `keep` piirtää SGR-koodien (`ESC[…m`) värit ja korostukset: perus- ja kirkkaat värit, 256 värin paletti, 24-bittiset värit sekä `bold`, `dim`, `italic`, `underline` ja `reverse` palautuksineen. Väri piirretään lokin tyylin (myös `LOG.stderr`) päälle, ja koodi on voimassa rivin loppuun.
  - Muut ohjaussekvenssit (kursorin siirrot, OSC-otsikot) poistetaan aina.
  - `NO_COLOR` poistaa myös säilytetyt värit, mutta korostukset jäävät.
  - Tulostemodaali (komento ilman `output`-määrettä) poistaa koodit aina.

### 13. Käyttäjän ohitukset ja asetukset

Kuten RLML D 3.2.

```
<XDG_CONFIG_HOME>/termplate/BACKUPTOOL/
    overrides/keys.termplate
    settings.termplate
```

- Ohitukset ovat paikkoja: olemassa oleva rivi korvautuu, uusi merkitään `new`, kirjoitusvirhe on latausvirhe.
- `settings`: kieli, `refresh`-kerroin, komentojen aikakatkaisu, hiiri, alueiden numerot.
- Teema on ohitustiedosto, esimerkiksi `overrides/theme.termplate`, jossa on `colors`- ja `panels`-osioita (luku 11).
- `vars`-lohko on paikattavissa, joten käyttäjä voi muuttaa muuttujien oletusarvoja (esim. `TARGET`).

### 14. Ajomallit

#### 14.1 Widget-tila

Yksi valikko, paneeli tai syöte; tulos stdoutiin kuten `dialog`.

```bash
choice=$(termplate menu ui.termplate MAIN_MENU) || exit
dir=$(termplate input ui.termplate TARGET_DIR)
```

- Paluuarvot: 0 valinta, 1 `back`, 2 `quit`, 3 muu virhe, signaalista 128 + signaali.
- Tiedostossa ei tarvitse olla `app`-lohkoa (luku 2).
- Listavalikko, jolla ei ole rivejä eikä `<VALIKKO>.EMPTY`-tekstiä, ei avaudu: paluuarvo on 1 eikä mitään tulosteta.
- Lomake (luku 8.1) tulostaa vahvistettaessa kenttien arvot omille riveilleen kenttäjärjestyksessä (`termplate panel ui.termplate LOGIN`); `then` jätetään ajamatta.
- Käyttöliittymä piirretään `/dev/tty`:hen, joten stdout jää tulokselle.
- **Termplate skriptin sisällä:** tiedostonimi `-` lukee Termplate-tiedoston stdinistä. Näin käyttöliittymän voi upottaa bash-skriptiin heredocina, ja koko ohjelma on yksi `.sh`-tiedosto:

```bash
choice=$(termplate menu - MAIN_MENU <<'EOF'
menu MAIN_MENU
- BACKUP "Varmuuskopioi"
- RESTORE "Palauta"
- QUIT "Lopeta"
EOF
) || exit
```

- Stdinistä luettaessa `include`-polut ja työhakemisto ovat nykyinen hakemisto. Sovelluksen nimi tulee `app`-lohkosta tai valitsimesta `--name NIMI`; ilman kumpaakaan käyttäjän ohituksia ja asetuksia ei ladata.
- Koprosessitila ei hyväksy `-`:tä, koska sen stdin on protokollalle. Siinä Termplate-tiedoston voi antaa prosessikorvauksena: `termplate serve <(cat <<'EOF' … EOF)`.

#### 14.2 Sovellustila

`termplate run app.termplate` pyörittää silmukkaa, ja kaikki toiminta tapahtuu komentojen kautta. Komennot voivat olla erillisiä skriptejä tai upotettuja (luku 6.1).

#### 14.3 Koprosessitila

Skripti ohjaa ja kuuntelee rivipohjaisella protokollalla.

```bash
coproc UI { termplate serve app.termplate; }
echo "set STATUS_TEXT Valmistellaan" >&"${UI[1]}"
while read -r event name value <&"${UI[0]}"; do
  case "$event $name" in
    "select BACKUP") ./backup.sh; echo "set USED 100" >&"${UI[1]}" ;;
    "closed MAIN_MENU") break ;;
  esac
done
```

- Isännältä: `set`, `open`, `close`, `log`, `refresh`, `busy on|off`, `quit`.
- Isännälle: `select VALIKKO KOHTA`, `input SYÖTE ARVO`, `submit LOMAKE` (lomakkeen jokaisen kentän `input`-tapahtuman jälkeen), `key TOIMINTO`, `closed VALIKKO`, `emit NIMI`.
- Arvo on rivin loppu; rivinvaihdot ja kenoviivat koodataan (`\n`, `\\`).
- Protokolla on oletuksena rivipohjainen. Valitsin `--json` vaihtaa sen JSON Linesiksi muita kieliä varten.

### 15. Komentojen ajo ja turvallisuus

- Komennot ajetaan `sh -c`:llä (tai `- shell`-määreen tulkilla, luku 6) ja välitetyt arvot ympäristömuuttujina; käyttäjän syötettä ei koskaan sijoiteta komentoriviin.
- **Upotetut skriptit** (luku 6.1) ajetaan väliaikaistiedostosta, jonka oikeudet ovat 600 ja joka poistetaan ajon jälkeen. Skriptin teksti on sovelluksen omaa koodia; käyttäjän arvot tulevat siihenkin vain ympäristömuuttujina.
- **Taustasuoritus:** komennot ja `$( … )`-lausekkeet ajetaan lapsiprosesseina käyttöliittymää pysäyttämättä. Lukijasäie ohjaa stdoutin ja stderrin lokiin, ja komennon käynnistänyt alue näyttää pyörivän merkin.
  - Eri komennot voivat olla käynnissä yhtä aikaa. Samaa komentoa ei ajeta kahdesti rinnakkain: uusi käynnistys ohitetaan ja lokiin tulee varoitus.
  - Jaksottaista päivitystä (`refresh`) ei aloiteta, jos edellinen on yhä kesken.
  - `back` keskeyttää uusimman nykyisestä kontekstista (päällimmäisestä modaalista tai fokusoidusta alueesta) käynnistetyn komennon (SIGTERM ja aikarajan jälkeen SIGKILL). Pääasettelussa, jossa `back` ei muuten sulje mitään, se keskeyttää uusimman käynnissä olevan komennon. Kun keskeytettävää ei ole, `back` toimii tavalliseen tapaan.
  - Keskeytetty komento ei jatka ketjuaan (`into`, `then`, `on_error`); lokiin tai tulosteeseen tulee teksti `TERMPLATE.CANCELLED`.
  - Kun ohjelma päättyy, käynnissä olevat komennot keskeytetään.
  - **Taustalaskenta:** paneelien kentät, valikkojen ehdot ja listojen lähteet, joissa on `$( … )`, lasketaan taustalla aikakatkaisulla (asetus `timeout`, oletus 10 s). Pelkät `$NIMI`-arvot lasketaan heti.
  - Kentät ja rivit näyttävät edelliset arvonsa, kunnes uudet valmistuvat. Ensimmäistä tulostaan odottava valikko näyttää tekstin `TERMPLATE.LOADING` (oletus `…`).
  - Valikkoa tai paneelia, jonka laskenta on kesken, ei lasketa uudelleen; jos jokin muuttui sillä välin, se lasketaan kerran lisää, kun edellinen valmistuu.
  - Valikon tai paneelin avaaminen (`open`, widget-tila) laskee arvot heti, koska widget-tila tarvitsee tiedon siitä, onko lista tyhjä.
  - Jokaisella komennolla on aikakatkaisu: komennon oma `- timeout AIKA` (esim. `30s`, `5m`), muuten asetus `timeout`. `interactive`- ja `window`-komennoilla aikakatkaisua ei ole, ja `- timeout` niissä on latausvirhe.
- Työhakemisto on sovellustiedoston kansio, ellei `app`-lohko toisin määrää.
- Pääte palautetaan ennalleen myös kaatuessa ja signaaleissa.
- Vuorovaikutteiset komennot (`less`, `vim`) saavat päätteen: Termplate vapauttaa sen ajon ajaksi (`run … interactive`), kuten Midnight Commanderin Ctrl-O.
- **Salaiset arvot:** `password`-syötteen arvo kulkee oletuksena komennon stdinissä (`- secret PASS stdin`) ja ympäristöön vain erikseen pyydettäessä (`- secret PASS env`). Arvoa ei kirjoiteta lokiin, historiaan eikä koprosessiprotokollaan.

#### 15.1 Erillinen ikkuna

```
command MONITOR
- run htop
- window popup
```

- `- window [tab|split|popup]` ajaa komennon uudessa ikkunassa, ja Termplate jatkaa omassaan. Komento on "käynnistä ja unohda": Termplate ei odota sitä eikä lue sen tulostetta tai paluuarvoa.
- **tmux** (ympäristössä `TMUX`): ikkuna avataan samaan istuntoon, joten tämä toimii myös SSH:n yli.
  - `tab` (oletus): `tmux new-window -d`; käyttäjä jää Termplateen.
  - `split`: `tmux split-window` Termplaten viereen.
  - `popup`: `tmux display-popup -E` (tmux 3.2+); vanhemmassa tmuxissa käytetään `tab`-tapaa.
  - Ikkuna sulkeutuu, kun komento päättyy, ellei käyttäjän tmux-asetus `remain-on-exit` ole päällä.
- **Ilman tmuxia:** macOS:ssä uusi Terminal-ikkuna (`osascript`), Linuxissa `$TERMINAL` tai `x-terminal-emulator`. `split` ja `popup` avaavat silloin tavallisen ikkunan.
- **Ei ikkunaa saatavilla** (SSH ilman tmuxia, Linux-konsoli, ei graafista päätettä): komento ajetaan `interactive`-tilassa samassa päätteessä, ja lokiin tulee varoitus.
- **Ympäristö:** uusi ikkuna saa tmux-palvelimen tai pääteohjelman ympäristön, ei Termplaten. Termplate kirjoittaa komennon ympäristön (luku 6) väliaikaiseen tiedostoon, jonka oikeudet ovat 600; ikkunassa ajettava kääre lukee ja poistaa sen ennen komentoa. Arvoja ei välitetä komentorivillä (esim. `tmux new-window -e`), koska ne näkyisivät `ps`-listauksessa. Työhakemisto välitetään (`-c`). Upotetun skriptin (luku 6.1) tiedosto välitetään samoin, ja kääre poistaa sen komennon jälkeen.
- **Rajoitukset**, jotka lataus tarkistaa: `window` ei käy yhteen määreiden `interactive`, `output`, `into` ja `timeout` kanssa, eikä `- secret NIMI stdin` ole mahdollinen (`secret … env` kulkee ympäristötiedostossa).
- `then` ja `on_error` riippuvat vain siitä, avautuiko ikkuna. `timeout`, `back` ja `stop` eivät koske ikkunassa ajettavaa komentoa.
- GNU screen (`screen -X screen`) sekä kittyn ja WezTermin etäohjaus voidaan lisätä myöhemmin samalla mallilla.

### 16. Isäntärajapinta ja Rust-arkkitehtuuri

- **termplate-core:** jäsennys, malli, latausvalidointi, asettelun ratkaisu, kenttien tunnistus, tekstit, näppäinkontekstit, piirtomalli. Ei I/O:ta.
- **termplate-term:** päätteen piirto ja syöte (crossterm), erotuspiirto.
- **termplate-shell:** shell-isäntä, komentojen ajo, protokolla ja CLI.
- **RLML-moottori** toteuttaa saman isäntärajapinnan RogueScriptillä.

```rust
trait Host {
    fn eval_value(&mut self, expr: &Expr, scope: &Scope) -> Value;
    fn eval_cond(&mut self, expr: &Expr, scope: &Scope) -> bool;
    fn eval_source(&mut self, expr: &Expr, scope: &Scope) -> Vec<Value>;
    fn run(&mut self, action: &Action, scope: &Scope) -> Outcome;
}
```

- Core jäsentää rakenteen ja jättää lausekkeet raakatekstiksi. Isäntä jäsentää ne latauksessa, joten lausekkeiden virheet ovat latausvirheitä.
- Samoin `run:`-alilohko (luku 6.1) kulkee coren läpi raakatekstinä kielitunnisteineen ja rivinumeroineen; isäntä valitsee tulkin ja tarkistaa syntaksin.

### 17. Avoimet kysymykset (koonti)

Ei avoimia kysymyksiä.

Ratkaistut: lomakkeet (luku 8.1), reunat ovat vain tyylissä (luku 11), widget-tiedostot ilman `app`-lohkoa (luku 2), komennon ympäristö ja `$( … )`-lausekkeiden pääsy kaikkiin muuttujiin (luku 6), kursoritoimintojen nimet (luku 9), polun täydennys ja alueiden numerointi (luvut 4 ja 8), erillinen ikkuna taustakomennoille (luku 15.1), yhden tiedoston sovellus (luku 2), upotetut skriptit ja tulkin valinta (luku 6.1), Termplate-tiedosto stdinistä (luku 14.1), säilytettyjen ANSI-värien piirto (luku 12), komennon aikakatkaisu (luku 15), peliohjaimen poisto (luku 9), teemat ohituksina eikä tiedostoina (luvut 11 ja 13).

### Liite B. Esimerkit

- `examples/backup`: luvun 2–12 esimerkkisovellus.
- `examples/filemanager`: kaksipaneelinen tiedostonhallinta Midnight Commanderin tapaan: listavalikot `filter`/`multi`/`keep`, paneelikohtaiset näppäinkontekstit, näppäinkomennot valituille riveille, `run … interactive` katseluun ja muokkaukseen.
- `examples/ollamaplate`: Ollaman käyttöliittymä: palvelin taustakomentona (`stop`), mallilista, ladatut mallit päivittyvässä paneelissa ja keskustelu asettelun syötteellä (`then`, `clear`), jonka vastaus virtaa lokiin.

### Liite A. Vastaavuus RLML:ään

| Termplate | RLML | Muutos |
| --- | --- | --- |
| `app` | `game` | vähemmän kenttiä |
| `vars` | kartan `vars:`, `game` | tyypitetyt muuttujat |
| `layout` | D 11.1 | valikko alueeseen |
| `panel` | D 11.5 | `refresh`, `bar`, `input` |
| `menu`, `list` | D 11.2 | `filter`, `multi` |
| `command` | RogueScriptin `command` | `run`, `run:`, `shell`, `env`, `output`, `window` |
| `input` | `ask`, luku- ja tekstisyöte | `validate`, lajit |
| `keys` | D 11.3 | hiirisidonnat, ei `pad`-osiota |
| kielitiedosto | D 10 | oletustekstit, upotus päätiedostoon |
| `log` | D 11.4 | stdout/stderr |
| `overrides/` | D 3.2 | XDG-polku |
