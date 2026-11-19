-- The Text tab's words in Russian (docs/UI_DESIGN.md, sections 6 "Text" and 9; the glossary is the
-- kit's, gz-ui/src/i18n/ru.lua). Revert, Go to Line, Take Theirs and the unsaved-edits sentences
-- are the PDA tab's and the graph tab's entries.
return {
  i18n = 1,
  lang = "ru",

  -- the toolbar
  { "Edit", "Текст", ctx = "text_mode" },
  { "Split", "Рядом", ctx = "text_mode" },
  { "Preview", "Просмотр", ctx = "text_mode" },
  { "The text, the text beside its preview, or the preview", "Текст, текст рядом с просмотром или только просмотр" },
  { "Find and replace in the text (Ctrl+F)", "Найти и заменить в тексте (Ctrl+F)" },
  { "The caret to a line (Ctrl+G)", "Курсор на строку (Ctrl+G)" },
  { "Go to Symbol", "Перейти к символу" },
  { "The file's functions (Ctrl+Shift+O)", "Функции файла (Ctrl+Shift+O)" },
  { "The file's sections (Ctrl+Shift+O)", "Секции файла (Ctrl+Shift+O)" },
  { "The file's headings (Ctrl+Shift+O)", "Заголовки файла (Ctrl+Shift+O)" },
  { "Line {n} {detail}", "Строка {n} {detail}" },
  { "Word Wrap, the encoding and the line endings the file is saved in", "Перенос строк, кодировка и концы строк, в которых сохраняется файл" },
  { "Word Wrap", "Перенос строк" },
  { "Save as UTF-8", "Сохранять в UTF-8" },
  { "Save as UTF-8 with BOM", "Сохранять в UTF-8 с BOM" },
  { "Save as Windows-1251", "Сохранять в Windows-1251" },
  { "Windows Line Endings (CRLF)", "Концы строк Windows (CRLF)" },
  { "Unix Line Endings (LF)", "Концы строк Unix (LF)" },
  { "A file of the game: the editor never writes it.", "Файл игры: редактор его никогда не записывает." },
  { "Read-only: a file of the game", "Только чтение: файл игры" },
  { "{path} - a file of the game, read-only", "{path} — файл игры, только чтение" },

  -- find, replace, go to line
  { "Match Case", "Учитывать регистр" },
  { "Tell capitals from small letters", "Различать заглавные и строчные буквы" },
  { "Whole Word", "Слово целиком" },
  { "Only where it stands as a word of its own", "Только там, где это отдельное слово" },
  { "{n} of {total}", "{n} из {total}" },
  { "{total} {total:match|matches}", "{total} {total:совпадение|совпадения|совпадений}" },
  { "Previous Match (Shift+F3)", "Предыдущее совпадение (Shift+F3)" },
  { "Next Match (F3)", "Следующее совпадение (F3)" },
  { "Replace what is found (Ctrl+H)", "Заменить найденное (Ctrl+H)" },
  { "Nothing is found", "Ничего не найдено" },
  { "Replace the current match and go to the next", "Заменить текущее совпадение и перейти к следующему" },
  { "Replace All", "Заменить все" },
  { "Replace every match, as one step Ctrl+Z takes back", "Заменить все совпадения одним шагом, который отменяет Ctrl+Z" },
  { "{n} {n:match|matches} replaced", "Заменено: {n} {n:совпадение|совпадения|совпадений}" },
  { "Line (1 to {n})", "Строка (от 1 до {n})" },
  { "Go", "Перейти" },

  -- the body and the status bar
  { "Reading the file...", "Чтение файла…" },
  { "The file cannot be opened", "Файл не открывается" },
  { "The tab's edits since the last save", "Правки вкладки с последнего сохранения" },
  { "Line {n}", "Строка {n}" },
  { "{lang} · {enc} · {eol} · Ln {line}, Col {col}", "{lang} · {enc} · {eol} · Стр {line}, Стлб {col}" },
  { "{lang} · {enc} · {eol} · Ln {line}, Col {col} · {n} {n:problem|problems}", "{lang} · {enc} · {eol} · Стр {line}, Стлб {col} · {n} {n:проблема|проблемы|проблем}" },
  { "Changed on disk too: {n} {n:place|places}. The tab keeps its own; Take Theirs takes the file's.", "Файл тоже изменился на диске: {n} {n:место|места|мест}. Вкладка оставила своё; «Взять с диска» берёт версию файла." },
  { "Nothing declares {name}: no script of the game or of the project has it.", "{name} нигде не объявлено: его нет ни в скриптах игры, ни в скриптах проекта." },
  { "The file could not be opened: {why}", "Файл не открылся: {why}" },
}
