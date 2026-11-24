package com.auroragba

import android.app.Activity
import android.app.Dialog
import android.content.res.Configuration
import android.graphics.drawable.ColorDrawable
import android.view.Gravity
import android.view.ViewGroup
import android.view.Window
import android.widget.FrameLayout
import android.widget.GridLayout
import android.widget.LinearLayout
import android.widget.ScrollView

/**
 * Menu do emulador (botão ☰) como painel que sobe de baixo, sobre o jogo: Shiny
 * Hunter em destaque (só em jogo suportado), velocidade acelerada liga/desliga,
 * ações rápidas em grade e, por último, Configurações.
 */
class MenuSheet(private val act: Activity, private val host: Host) {
    private val ui = Ui(act)
    private val dialog = Dialog(act).apply {
        requestWindowFeature(Window.FEATURE_NO_TITLE)
        setCanceledOnTouchOutside(true)
        setOnDismissListener { host.onMenuClosed() }
    }

    fun show() {
        val content = ui.box(true, 16).apply {
            background = roundTop(ui)
            setPadding(ui.dp(20), ui.dp(12), ui.dp(20), ui.dp(24))
        }
        // Alça do painel.
        content.addView(android.view.View(act).apply {
            background = ui.rounded(Pal.OFF, 2)
        }, LinearLayout.LayoutParams(ui.dp(40), ui.dp(4)).apply { gravity = Gravity.CENTER_HORIZONTAL })

        content.addView(header())
        if (host.hunterAvailable) content.addView(hunterCard())
        content.addView(fastRow())
        content.addView(actions())
        content.addView(ui.navRow(R.drawable.ic_gear, host.tr(R.string.settings_title), null) {
            dialog.dismiss()
            host.onSettings()
        }.apply { background = ui.touchable(ui.rounded(0x00000000, 14, Pal.BORDER)); minimumHeight = ui.dp(56) })

        dialog.setContentView(ScrollView(act).apply { addView(content) })
        dialog.window?.apply {
            setBackgroundDrawable(ColorDrawable(0))
            setGravity(Gravity.BOTTOM)
            // Paisagem: painel estreito no centro, pra não cobrir a tela inteira.
            val landscape = act.resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE
            setLayout(if (landscape) ui.dp(520) else ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
        }
        dialog.show()
    }

    /** Executa [action] depois de fechar o menu (a maioria abre outro diálogo). */
    private fun then(action: () -> Unit): () -> Unit = {
        dialog.dismiss()
        action()
    }

    private fun header() = ui.box(false, 12).apply {
        gravity = Gravity.CENTER_VERTICAL
        val sub = if (host.menuPauses) R.string.menu_paused else R.string.menu_running_link
        addView(ui.box(true, 2).apply {
            addView(ui.text(host.gameTitle ?: host.tr(R.string.menu_no_rom), 18f, bold = true))
            if (host.gameTitle != null) addView(ui.text(host.tr(sub), 13f, Pal.MUTED))
        }, ui.lp(0, Ui.WRAP, 1f))
        addView(FrameLayout(act).apply {
            background = ui.touchable(ui.rounded(Pal.SURFACE2, 22))
            contentDescription = host.tr(R.string.menu_close)
            addView(ui.icon(R.drawable.ic_close, Pal.TEXT, 20).apply {
                layoutParams = FrameLayout.LayoutParams(ui.dp(20), ui.dp(20), Gravity.CENTER)
            })
            setOnClickListener { dialog.dismiss() }
        }, LinearLayout.LayoutParams(ui.dp(44), ui.dp(44)))
    }

    private fun hunterCard() = ui.box(false, 14).apply {
        gravity = Gravity.CENTER_VERTICAL
        background = ui.touchable(ui.rounded(Pal.ACCENT, 16))
        setPadding(ui.dp(16), ui.dp(16), ui.dp(16), ui.dp(16))
        addView(ui.icon(R.drawable.ic_sparkle, Pal.ON_ACCENT, 28))
        addView(ui.box(true, 2).apply {
            addView(ui.text(host.tr(R.string.hunter_title), 16f, Pal.ON_ACCENT, bold = true))
            addView(ui.text(host.tr(R.string.hunter_card_sub), 13f, Pal.ON_ACCENT))
        }, ui.lp(0, Ui.WRAP, 1f))
        val run = then(host::onHunter)
        setOnClickListener { run() }
    }

    private fun fastRow() = ui.box(false, 12).apply {
        gravity = Gravity.CENTER_VERTICAL
        background = ui.rounded(Pal.SURFACE2, 16)
        setPadding(ui.dp(16), ui.dp(12), ui.dp(16), ui.dp(12))
        addView(ui.icon(R.drawable.ic_fast, Pal.TEXT, 24))
        val sub = ui.text(fastSubtitle(host.fastOn), 12f, Pal.MUTED)
        addView(ui.box(true, 2).apply {
            addView(ui.text(host.tr(R.string.menu_fast), 15f, bold = true))
            addView(sub)
        }, ui.lp(0, Ui.WRAP, 1f))
        val sw = ui.switch(host.fastOn) { on ->
            host.setFast(on)
            sub.text = fastSubtitle(on)
        }
        addView(sw)
        setOnClickListener { sw.toggle() }
    }

    private fun fastSubtitle(on: Boolean) =
        host.tr(if (on) R.string.menu_fast_on else R.string.menu_fast_off, host.fastMultiplier)

    /** Grade 2×3 das ações rápidas. */
    private fun actions() = GridLayout(act).apply {
        columnCount = 2
        val items = listOf(
            Triple(R.drawable.ic_save, R.string.menu_save_state, host::onSaveState),
            Triple(R.drawable.ic_load, R.string.menu_load_state, host::onLoadState),
            Triple(R.drawable.ic_folder, R.string.menu_load_rom, host::onLoadRom),
            Triple(R.drawable.ic_reset, R.string.menu_reset_rom, host::onResetRom),
            Triple(R.drawable.ic_camera, R.string.menu_screenshot_plain, host::onScreenshot),
            Triple(R.drawable.ic_link, R.string.menu_link_cable, host::onLink),
        )
        val gap = ui.dp(5)
        items.forEach { (iconRes, label, action) ->
            val tile = ui.box(false, 10).apply {
                gravity = Gravity.CENTER_VERTICAL
                background = ui.touchable(ui.rounded(Pal.SURFACE2, 14))
                minimumHeight = ui.dp(64)
                setPadding(ui.dp(12), ui.dp(12), ui.dp(12), ui.dp(12))
                addView(ui.icon(iconRes, Pal.ACCENT, 22))
                addView(ui.text(host.tr(label), 14f, bold = true))
                val run = then(action)
                setOnClickListener { run() }
            }
            addView(tile, GridLayout.LayoutParams(
                GridLayout.spec(GridLayout.UNDEFINED), GridLayout.spec(GridLayout.UNDEFINED, 1f),
            ).apply {
                width = 0
                setMargins(gap, gap, gap, gap)
            })
        }
    }
}

/** Fundo do painel: superfície com só os cantos de cima arredondados. */
private fun roundTop(ui: Ui) = android.graphics.drawable.GradientDrawable().apply {
    setColor(Pal.SURFACE)
    val r = ui.dp(20).toFloat()
    cornerRadii = floatArrayOf(r, r, r, r, 0f, 0f, 0f, 0f)
}
