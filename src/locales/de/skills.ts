/** Skills, the comparison with another player, goals and practice suggestions. */
export const skills: Record<string, string> = {
  // pattern names stay English, as in the game
  Stream: "Stream",
  Jumpstream: "Jumpstream",
  Handstream: "Handstream",
  Chordjack: "Chordjack",
  Jacks: "Jacks",

  Compare: "Vergleichen",
  "Skills come from your osu!mania plays": "Skills kommen aus deinen osu!mania-Plays",
  "Fumen rates your osu!mania passes the way Etterna does. Once you've passed some 4K to 7K charts that are in your library, you'll see here what you're good at.":
    "Fumen bewertet deine osu!mania-Passes so, wie Etterna es tut. Sobald du ein paar 4K- bis 7K-Charts aus deiner Bibliothek gepasst hast, siehst du hier, was du gut kannst.",
  "{n} pass rated|{n} passes rated": "{n} Pass bewertet|{n} Passes bewertet",
  "still filling up": "füllt sich noch",
  "{n} chart passed|{n} charts passed": "{n} Chart gepasst|{n} Charts gepasst",
  "Every pass is rated with Etterna's calculator, MinaCalc: the chart at the rate you played it, for your accuracy as Etterna counts it (Wife, worked out from your judgements). That's what the play is worth in each skillset. A skillset blends all your plays there the way Etterna does: your best count the most, and it takes a fair number of good plays to get up to your level. Overall blends the seven. As in Etterna, a chart counts with its two best rates (rate edits of it are the same chart), passes only, and not with mods that change what you play, like Hold Off.":
    "Jeder Pass wird mit Etternas Rechner MinaCalc bewertet: das Chart bei der Rate, mit der du es gespielt hast, für deine Accuracy, wie Etterna sie zählt (Wife, aus deinen Judgements berechnet). Das ist, was das Play in jedem Skillset wert ist. Ein Skillset fasst alle deine Plays darin so zusammen, wie Etterna es tut: Deine besten zählen am meisten, und es braucht eine ordentliche Zahl guter Plays, bis es auf deinem Niveau ist. Overall fasst die sieben zusammen. Wie in Etterna zählt ein Chart mit seinen zwei besten Rates (Rate-Edits davon sind dasselbe Chart), nur Passes, und nicht mit Mods, die ändern, was du spielst, wie Hold Off.",
  "MinaCalc rates some charts higher than they play. A pass worth far more than your skill is marked as looking overrated, and you can leave such a chart out. Fumen also looks for parts no hand plays as written, like jacks or trills past a finger's speed (vibro): where they're a small part of a chart they're cut out before rating, otherwise the pass doesn't count. Its rules come from mania-tracker.":
    "MinaCalc bewertet manche Charts höher, als sie sich spielen. Ein Pass, der weit mehr wert ist als dein Skill, wird als wohl überbewertet markiert, und du kannst so ein Chart rausnehmen. Fumen sucht außerdem nach Stellen, die keine Hand so spielt, wie sie geschrieben sind, etwa Jacks oder Trills schneller, als ein Finger kann (Vibro): Sind sie nur ein kleiner Teil des Charts, werden sie vor der Bewertung herausgeschnitten, sonst zählt der Pass nicht. Die Regeln dafür stammen von mania-tracker.",
  "parts no hand plays as written": "Stellen, die keine Hand so spielt",
  "notes MinaCalc can't follow": "Noten, denen MinaCalc nicht folgen kann",
  "Fumen left out your passes on these charts at the rates shown: they have parts no hand plays as written (vibro), or notes MinaCalc can't follow, like jumptrills or chords split into notes a few ms apart.":
    "Fumen hat deine Passes auf diesen Charts bei den gezeigten Rates rausgenommen: Sie haben Stellen, die keine Hand so spielt, wie sie geschrieben sind (Vibro), oder Noten, denen MinaCalc nicht folgen kann, etwa Jumptrills oder Akkorde, die in Noten ein paar ms auseinander zerlegt sind.",
  "Parts no hand plays as written were cut out before rating this pass": "Stellen, die keine Hand so spielt, wurden vor der Bewertung dieses Passes herausgeschnitten",
  "This chart has parts no hand plays as written at this rate, like jacks or trills past a finger's speed (vibro), so this pass doesn't count for your skills.":
    "Dieses Chart hat bei dieser Rate Stellen, die keine Hand so spielt, wie sie geschrieben sind, etwa Jacks oder Trills schneller, als ein Finger kann (Vibro). Darum zählt dieser Pass nicht für deine Skills.",
  "This chart is built from notes MinaCalc can't follow, like jumptrills or chords split into notes a few ms apart, so this pass doesn't count for your skills.":
    "Dieses Chart besteht aus Noten, denen MinaCalc nicht folgen kann, etwa Jumptrills oder Akkorde, die in Noten ein paar ms auseinander zerlegt sind. Darum zählt dieser Pass nicht für deine Skills.",
  "Parts no hand plays as written were cut out before rating it.": "Stellen, die keine Hand so spielt, wurden vor der Bewertung herausgeschnitten.",
  "Worth far more than this skill. MinaCalc rates some charts too high: open the list to leave one out.":
    "Weit mehr wert als dieser Skill. MinaCalc bewertet manche Charts zu hoch: Öffne die Liste, um eins rauszunehmen.",
  "{n} looks overrated|{n} look overrated": "{n} wirkt überbewertet|{n} wirken überbewertet",
  "Looks overrated: worth far more than your {skill}": "Wirkt überbewertet: weit mehr wert als dein {skill}",
  "Leave out of the skills": "Aus den Skills nehmen",
  "Left out of the skills: {name}": "Aus den Skills genommen: {name}",
  Undo: "Rückgängig",
  "Left out of the skills": "Aus den Skills genommen",
  "These charts count for nothing here, at any rate, and aren't suggested to practise.":
    "Diese Charts zählen hier nicht, bei keiner Rate, und werden auch nicht zum Üben vorgeschlagen.",
  "Count again": "Wieder zählen",
  "You left this chart out of your skills, so this pass doesn't count.": "Du hast dieses Chart aus deinen Skills genommen, darum zählt dieser Pass nicht.",
  "This pass is worth far more than your {skill}. MinaCalc rates some charts too high. If this is one, leave it out.":
    "Dieser Pass ist weit mehr wert als dein {skill}. MinaCalc bewertet manche Charts zu hoch. Wenn das so eins ist, nimm es raus.",
  "Leave out": "Rausnehmen",
  "Since {when}": "Seit {when}",
  "best play {value}": "bestes Play {value}",
  "{n} play carries it|{n} plays carry it": "{n} Play trägt ihn|{n} Plays tragen ihn",
  "Plays worth more than this skill: only they lift it, the rest adds nothing": "Plays, die mehr wert sind als dieser Skill: Nur sie heben ihn, der Rest trägt nichts bei",
  "Set a goal for {skill}": "Ziel für {skill} setzen",
  "{wife} as Etterna counts it": "{wife}, wie Etterna es zählt",
  "What this play is worth in {skill}": "Was dieses Play in {skill} wert ist",

  // comparing
  "{name} has no passes rated here": "{name} hat hier keine bewerteten Passes",
  "{name}: {value} from {plays}": "{name}: {value} aus {plays}",
  "{n} pass|{n} passes": "{n} Pass|{n} Passes",
  "{name}'s best in {skill}": "Das Beste von {name} in {skill}",
  Yours: "Deine",
  "Comparing with {name}": "Vergleiche mit {name}",
  Stop: "Stopp",
  Unranked: "Ohne Rang",
  "{n} chart no mirror had|{n} charts no mirror had": "{n} Chart hatte kein Mirror|{n} Charts hatte kein Mirror",
  "Compared with {name}": "Verglichen mit {name}",
  "Other player": "Andere Person",
  "End the comparison": "Vergleich beenden",
  "osu! player name": "Name auf osu!",
  "Their top plays, first places, recent passes and most played osu!mania charts come from osu!; charts you don't have are read from the mirrors.":
    "Ihre Top-Plays, ersten Plätze, neuesten Passes und meistgespielten osu!mania-Charts kommen von osu!; Charts, die du nicht hast, werden von den Mirrors gelesen.",
  "From {name}'s plays": "Aus den Plays von {name}",
  "Add to osu!lazer…": "Zu osu!lazer hinzufügen…",
  "Like it in your library": "Ähnliches in deiner Bibliothek",
  "Nothing much like it": "Nichts wirklich Ähnliches",
  "Not in your library: add it": "Nicht in deiner Bibliothek: hinzufügen",
  "not in your library": "nicht in deiner Bibliothek",
  "in your library": "in deiner Bibliothek",

  // goals
  "More for this goal": "Mehr zu diesem Ziel",
  "{value} now": "jetzt {value}",
  "Not passed yet": "Noch nicht gepasst",
  "Nothing yet": "Noch nichts",
  "Set {when}": "Gesetzt {when}",
  "reached {when}": "erreicht {when}",
  Goals: "Ziele",
  "Something to work toward: a skill, an accuracy on a chart, or your pp or rank. Fumen ticks it off when your plays get there.":
    "Etwas, auf das du hinarbeitest: ein Skill, eine Accuracy auf einem Chart oder deine pp oder dein Rang. Fumen hakt es ab, sobald deine Plays dort ankommen.",
  "Hide reached": "Erreichte ausblenden",
  "Reached {n}": "Erreicht {n}",
  "Open beatmap": "Beatmap öffnen",
  "Show {keys}K skills": "{keys}K-Skills zeigen",
  "Delete goal": "Ziel löschen",
  "Couldn't delete the goal: {error}": "Das Ziel ließ sich nicht löschen: {error}",
  "Set a goal…": "Ziel setzen…",
  "{ruleset} rank": "{ruleset}-Rang",
  "{acc} at {rate}× or faster": "{acc} bei {rate}× oder schneller",

  // practice
  "Practice next": "Als Nächstes üben",
  "Charts in your library a little above your best plays in your weakest skillsets, and mostly about them: hard enough to move them, close enough to pass. Where 1× doesn't land there, a rate does.":
    "Charts aus deiner Bibliothek knapp über deinen besten Plays in deinen schwächsten Skillsets, und vor allem genau darin: schwer genug, um sie zu verbessern, nah genug, um sie zu passen. Wo 1× nicht passt, tut es eine Rate.",
  "Fumen is still working out the MSD of your charts. Suggestions show up here once that's done.":
    "Fumen berechnet gerade noch die MSD deiner Charts. Vorschläge erscheinen hier, sobald das fertig ist.",
  "Weakest only": "Nur die schwächsten",
  "All {n} skillsets": "Alle {n} Skillsets",
  "No passes yet": "Noch keine Passes",
  "{lo} to {hi}": "{lo} bis {hi}",
  "your best {acc} at {rate}×": "dein Bestes {acc} bei {rate}×",
  "your best {acc}": "dein Bestes {acc}",
  played: "gespielt",
  "not passed yet": "noch nicht gepasst",
  "aiming at {band}": "Ziel {band}",
  "Nothing in your library that's mostly {skill} reaches {band}, not even at 0.75× to 1.5×.":
    "Nichts in deiner Bibliothek, das vor allem {skill} ist, erreicht {band}, nicht mal bei 0,75× bis 1,5×.",
  "MSD in {skill} at {rate}×": "MSD in {skill} bei {rate}×",
  "MSD in {skill}": "MSD in {skill}",
  "Star rating": "Sternebewertung",
  "{mod} with its speed set to {rate}×": "{mod} mit Geschwindigkeit auf {rate}×",
  new: "neu",
  "A pass at {wife} Wife here takes {skill} from {from} to {to}": "Ein Pass mit {wife} Wife hier bringt {skill} von {from} auf {to}",
  "A pass at {wife} Wife here starts {skill} at {to}": "Ein Pass mit {wife} Wife hier startet {skill} bei {to}",

  // a skill's graph
  "Graph over time": "Verlauf als Graph",
  "Hide the graph": "Graph ausblenden",
  "Nothing to show yet.": "Noch nichts zu zeigen.",
  "{change} in this time": "{change} in dieser Zeit",
  "About the same in this time": "In dieser Zeit etwa gleich",
  "Nothing moved in this time.": "In dieser Zeit hat sich nichts bewegt.",
  "Time range": "Zeitraum",
  "90 days": "90 Tage",
  Year: "Jahr",
  "{skill}: {from} to {to}": "{skill}: {from} bis {to}",

  // on the score page
  "This pass is worth {value} overall, at {wife} as Etterna counts it.": "Dieser Pass ist {value} Overall wert, bei {wife}, wie Etterna es zählt.",
  "Open Skills": "Skills öffnen",
  "No skill went up this time.": "Diesmal ist kein Skill gestiegen.",

  // radar
  "Goal: {value}": "Ziel: {value}",
  "Skill per skillset": "Skill pro Skillset",
  You: "Du",
  Now: "Jetzt",
  "90 days ago": "Vor 90 Tagen",
  Goal: "Ziel",
};
