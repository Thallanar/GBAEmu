package com.auroragba

import android.content.Context
import android.content.res.Configuration
import java.util.Locale

/**
 * Idioma do app. Os textos ficam nos `strings.xml` de `res/`: inglês em `values/`
 * (o padrão — um sistema em francês cai nele), português em `values-pt/` e
 * espanhol em `values-es/`.
 *
 * A escolha do usuário fica nas prefs (`""` = automático, segue o sistema — e,
 * no Android 13+, a escolha por app feita nas configurações do sistema). Os
 * textos são lidos de um [Context] com o idioma aplicado ([context]), refeito na
 * hora da troca: recriar a Activity derrubaria a emulação em andamento.
 */
object L10n {
    const val KEY = "app_language"
    // Mesmo arquivo de prefs do MainActivity (`PREFS`).
    private const val PREFS = "auroragba"

    /** Idiomas oferecidos (tag BCP 47; `""` = automático) e o nome de cada um nele mesmo. */
    val OPTIONS = arrayOf(
        "" to "",
        "pt-BR" to "Português (Brasil)",
        "en" to "English",
        "es" to "Español",
    )

    fun load(ctx: Context): String =
        ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY, "") ?: ""

    fun save(ctx: Context, tag: String) =
        ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY, tag).apply()

    /** [base] com o idioma [tag] aplicado; `""` devolve o próprio [base] (idioma do sistema). */
    fun context(base: Context, tag: String): Context {
        if (tag.isEmpty()) return base
        val cfg = Configuration(base.resources.configuration)
        cfg.setLocale(Locale.forLanguageTag(tag))
        return base.createConfigurationContext(cfg)
    }

    /** Locale efetivo dos textos de [ctx] (datas e números seguem o idioma do app). */
    fun locale(ctx: Context): Locale = ctx.resources.configuration.locales[0]
}
