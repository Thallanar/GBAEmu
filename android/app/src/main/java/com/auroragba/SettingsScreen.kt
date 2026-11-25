package com.auroragba

import android.app.Activity
import android.app.AlertDialog
import android.app.Dialog
import android.graphics.Typeface
import android.view.Gravity
import android.view.KeyEvent
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView

/**
 * Configurações em tela cheia, com navegação em páginas (seta/voltar desempilha):
 * Pasta de ROMs, Áudio, Velocidade, Interface, Idioma e Sobre. O jogo fica
 * pausado enquanto ela está aberta (o host cuida disso).
 */
class SettingsScreen(private val act: Activity, private val host: Host) {
    private enum class Page { ROOT, ROMS, AUDIO, SPEED, INTERFACE, LANGUAGE, ABOUT }

    private val ui = Ui(act)
    private val stack = ArrayDeque(listOf(Page.ROOT))
    private var closingToMenu = false

    private val title = ui.text("", 22f, bold = true)
    private val body = ui.box(true, 12).apply {
        setPadding(ui.dp(16), ui.dp(12), ui.dp(16), ui.dp(32))
    }
    private val scroll = ScrollView(act).apply { addView(body) }

    private val dialog = Dialog(act, android.R.style.Theme_Black_NoTitleBar_Fullscreen).apply {
        setOnKeyListener { _, keyCode, event ->
            if (keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) {
                back()
                true
            } else {
                keyCode == KeyEvent.KEYCODE_BACK
            }
        }
        setOnDismissListener { host.onSettingsClosed(closingToMenu) }
    }

    fun show() {
        val backButton = FrameLayout(act).apply {
            background = ui.touchable(ui.rounded(0x00000000, 22))
            contentDescription = host.tr(R.string.back)
            addView(ui.icon(R.drawable.ic_back, Pal.TEXT, 22).apply {
                layoutParams = FrameLayout.LayoutParams(ui.dp(22), ui.dp(22), Gravity.CENTER)
            })
            setOnClickListener { back() }
        }
        val header = ui.box(false, 8).apply {
            gravity = Gravity.CENTER_VERTICAL
            setPadding(ui.dp(12), ui.dp(16), ui.dp(12), ui.dp(8))
            addView(backButton, LinearLayout.LayoutParams(ui.dp(44), ui.dp(44)))
            addView(title)
        }
        val root = ui.box(true).apply {
            setBackgroundColor(Pal.BG)
            addView(header)
            addView(scroll, ui.lp(Ui.MATCH, 0, 1f))
        }
        dialog.setContentView(root)
        render()
        dialog.show()
    }

    /** Redesenha a página atual (após trocar pasta, idioma, shader…). */
    fun refresh() {
        if (dialog.isShowing) render()
    }

    private fun open(page: Page) {
        stack.addLast(page)
        render()
        scroll.scrollTo(0, 0)
    }

    private fun back() {
        if (stack.size > 1) {
            stack.removeLast()
            render()
        } else {
            closingToMenu = true
            dialog.dismiss()
        }
    }

    private fun render() {
        body.removeAllViews()
        when (stack.last()) {
            Page.ROOT -> root()
            Page.ROMS -> roms()
            Page.AUDIO -> audio()
            Page.SPEED -> speed()
            Page.INTERFACE -> iface()
            Page.LANGUAGE -> language()
            Page.ABOUT -> about()
        }
    }

    private fun t(id: Int, vararg args: Any) = host.tr(id, *args)

    // ── Páginas ──────────────────────────────────────────────────────────────

    private fun root() {
        title.text = t(R.string.settings_title)
        body.addView(ui.navRow(R.drawable.ic_folder, t(R.string.set_roms), t(R.string.set_roms_sub)) { open(Page.ROMS) })
        body.addView(ui.navRow(R.drawable.ic_volume, t(R.string.set_audio), t(R.string.set_audio_sub)) { open(Page.AUDIO) })
        body.addView(ui.navRow(R.drawable.ic_fast, t(R.string.set_speed), t(R.string.set_speed_sub, host.fastMultiplier)) { open(Page.SPEED) })
        body.addView(ui.navRow(R.drawable.ic_gamepad, t(R.string.set_interface), t(R.string.set_interface_sub)) { open(Page.INTERFACE) })
        body.addView(ui.navRow(R.drawable.ic_globe, t(R.string.language_title), languageName(host.languageTag())) { open(Page.LANGUAGE) })
        body.addView(ui.navRow(R.drawable.ic_info, t(R.string.set_about), t(R.string.set_about_sub)) { open(Page.ABOUT) })
        body.addView(ui.text("AuroraGBA ${host.versionName()}", 12f, Pal.FAINT).apply {
            gravity = Gravity.CENTER
            setPadding(0, ui.dp(16), 0, 0)
        }, ui.lp())
    }

    private fun roms() {
        title.text = t(R.string.set_roms)
        val folder = host.romFolderName()
        body.addView(ui.card(14).apply {
            addView(ui.sectionLabel(t(R.string.roms_current)))
            addView(ui.box(false, 12).apply {
                gravity = Gravity.CENTER_VERTICAL
                addView(ui.icon(R.drawable.ic_folder, Pal.ACCENT, 28))
                addView(ui.titled(
                    folder ?: t(R.string.roms_none),
                    if (folder != null) host.plural(R.plurals.roms_count, host.romCount()) else null,
                ), ui.lp(0, Ui.WRAP, 1f))
            })
            addView(ui.accentButton(t(if (folder != null) R.string.roms_change else R.string.roms_choose)) {
                host.pickRomFolder()
            })
        })
        body.addView(ui.card(12).apply {
            addView(ui.sectionLabel(t(R.string.roms_layout_title)))
            val name = folder ?: "ROMs"
            val g = t(R.string.roms_tree_game)
            addView(ui.text("$name/\n├─ $g.gba\n└─ saves/\n   ├─ $g.sav\n   └─ $g.ss1 … ss8", 13f, 0xFFD6D6DD.toInt()).apply {
                typeface = Typeface.MONOSPACE
                background = ui.rounded(Pal.BG, 10)
                setPadding(ui.dp(14), ui.dp(12), ui.dp(14), ui.dp(12))
            })
            addView(ui.text(t(R.string.roms_layout_note), 13f, Pal.MUTED))
        })
    }

    private fun audio() {
        title.text = t(R.string.set_audio)
        body.addView(ui.switchRow(t(R.string.audio_sound), t(R.string.audio_sound_sub), host.soundOn) {
            host.soundOn = it
        })
        body.addView(ui.sliderRow(t(R.string.audio_volume), host.volume, 0, 100, 5, "%") { host.volume = it })
        body.addView(ui.card(12).apply {
            addView(ui.titled(t(R.string.audio_ff), t(R.string.audio_ff_sub)))
            addView(ui.segmented(
                listOf(t(R.string.audio_ff_mute), t(R.string.audio_ff_fast)),
                if (host.ffMuted) 0 else 1,
            ) { host.ffMuted = it == 0 })
        })
    }

    private fun speed() {
        title.text = t(R.string.set_speed)
        body.addView(ui.card(14).apply {
            addView(ui.titled(t(R.string.menu_fast), t(R.string.speed_desc)))
            val options = intArrayOf(2, 4, 8)
            val row = ui.box(false, 8)
            val buttons = options.map { x -> ui.text("$x×", 18f, bold = true).apply { gravity = Gravity.CENTER; minHeight = ui.dp(56) } }
            fun paint() = buttons.forEachIndexed { i, b ->
                val on = options[i] == host.fastMultiplier
                b.background = ui.touchable(ui.rounded(if (on) Pal.ACCENT_SOFT else Pal.SURFACE2, 12, if (on) Pal.ACCENT else 0xFF33333D.toInt(), 2))
                b.setTextColor(if (on) Pal.ACCENT else Pal.TEXT)
            }
            buttons.forEachIndexed { i, b ->
                b.setOnClickListener { host.fastMultiplier = options[i]; paint() }
                row.addView(b, ui.lp(0, Ui.WRAP, 1f))
            }
            paint()
            addView(row)
        })
        body.addView(ui.box(false, 12).apply {
            background = ui.rounded(Pal.SURFACE, 16)
            setPadding(ui.dp(16), ui.dp(16), ui.dp(16), ui.dp(16))
            addView(ui.icon(R.drawable.ic_info, Pal.MUTED, 22))
            addView(ui.text(t(R.string.speed_note), 13f, Pal.MUTED), ui.lp(0, Ui.WRAP, 1f))
        })
    }

    private fun iface() {
        title.text = t(R.string.set_interface)
        body.addView(ui.sectionLabel(t(R.string.iface_onscreen)))
        body.addView(ui.card(14).apply {
            addView(ui.text(t(R.string.pad_model), 16f, bold = true))
            val models = listOf(
                Triple(PadModel.CLASSIC, R.drawable.ic_pad_classic, R.string.pad_classic),
                Triple(PadModel.STICK, R.drawable.ic_pad_stick, R.string.pad_stick),
                Triple(PadModel.ARROWS, R.drawable.ic_pad_arrows, R.string.pad_arrows),
            )
            val row = ui.box(false, 8)
            val cards = models.map { (_, iconRes, label) ->
                ui.box(true, 8).apply {
                    gravity = Gravity.CENTER
                    minimumHeight = ui.dp(96)
                    addView(ui.icon(iconRes, Pal.TEXT, 40))
                    addView(ui.text(t(label), 14f, bold = true).apply { gravity = Gravity.CENTER })
                }
            }
            fun paint() = cards.forEachIndexed { i, c ->
                val on = models[i].first == host.padModel
                c.background = ui.touchable(ui.rounded(if (on) Pal.ACCENT_SOFT else Pal.SURFACE2, 14, if (on) Pal.ACCENT else 0xFF33333D.toInt(), 2))
                val color = if (on) Pal.ACCENT else Pal.TEXT
                (c.getChildAt(0) as android.widget.ImageView).imageTintList = android.content.res.ColorStateList.valueOf(color)
                (c.getChildAt(1) as android.widget.TextView).setTextColor(color)
            }
            cards.forEachIndexed { i, c ->
                c.setOnClickListener { host.padModel = models[i].first; paint() }
                row.addView(c, ui.lp(0, Ui.WRAP, 1f))
            }
            paint()
            addView(row)
        })
        body.addView(ui.navRow(null, t(R.string.iface_edit_layout), t(R.string.iface_edit_layout_sub)) {
            // O editor desenha por cima do jogo: as Configurações saem da frente
            // e voltam (nesta mesma página) quando ele fecha.
            dialog.hide()
            host.openLayoutEditor { dialog.show() }
        })
        body.addView(ui.sliderRow(t(R.string.pad_size), host.padSize, 70, 130, 5, "%") { host.padSize = it })
        body.addView(ui.sliderRow(t(R.string.pad_opacity), host.padOpacity, 10, 60, 1, "%") { host.padOpacity = it })
        body.addView(ui.switchRow(t(R.string.pad_haptics), t(R.string.pad_haptics_sub), host.padHaptics) {
            host.padHaptics = it
        })
        body.addView(ui.switchRow(t(R.string.pad_hide), t(R.string.pad_hide_sub), host.padHideWithController) {
            host.padHideWithController = it
        })
        body.addView(ui.sectionLabel(t(R.string.iface_video)))
        body.addView(ui.group(
            ui.valueRow(t(R.string.shader_title), host.shaderLabel()) { host.openShader(::refresh) },
            ui.valueRow(t(R.string.upscale_title), host.upscaleLabel()) { host.openUpscale(::refresh) },
        ))
        body.addView(ui.sectionLabel(t(R.string.iface_pad)))
        body.addView(ui.group(ui.valueRow(t(R.string.iface_remap), null) { host.openRemap() }))
    }

    private fun language() {
        title.text = t(R.string.language_title)
        val current = host.languageTag()
        val rows = L10n.OPTIONS.map { (tag, _) ->
            val on = tag == current
            ui.box(false, 12).apply {
                gravity = Gravity.CENTER_VERTICAL
                background = ui.touchable(ui.rounded(0x00000000, 0))
                minimumHeight = ui.dp(64)
                setPadding(ui.dp(16), 0, ui.dp(16), 0)
                addView(ui.titled(languageName(tag), if (tag.isEmpty()) t(R.string.language_auto_sub) else null), ui.lp(0, Ui.WRAP, 1f))
                // Rádio: anel com miolo azul quando selecionado.
                addView(FrameLayout(act).apply {
                    background = ui.rounded(0x00000000, 11, if (on) Pal.ACCENT else 0xFF5A5A66.toInt(), 2)
                    if (on) addView(android.view.View(act).apply { background = ui.rounded(Pal.ACCENT, 5) },
                        FrameLayout.LayoutParams(ui.dp(10), ui.dp(10), Gravity.CENTER))
                }, LinearLayout.LayoutParams(ui.dp(22), ui.dp(22)))
                setOnClickListener {
                    host.setLanguage(tag)
                    render() // a página inteira já sai no idioma novo
                }
            }
        }
        body.addView(ui.group(*rows.toTypedArray()))
        body.addView(ui.text(t(R.string.lang_note), 13f, Pal.MUTED).apply { setPadding(ui.dp(8), 0, ui.dp(8), 0) })
    }

    private fun about() {
        title.text = t(R.string.set_about)
        body.addView(ui.card(6).apply {
            gravity = Gravity.CENTER_HORIZONTAL
            setPadding(ui.dp(16), ui.dp(24), ui.dp(16), ui.dp(24))
            addView(android.widget.ImageView(act).apply {
                setImageResource(R.mipmap.ic_launcher)
            }, LinearLayout.LayoutParams(ui.dp(64), ui.dp(64)))
            addView(ui.text("AuroraGBA", 20f, bold = true).apply { gravity = Gravity.CENTER })
            addView(ui.text(t(R.string.about_version, host.versionName()), 14f, Pal.MUTED).apply { gravity = Gravity.CENTER })
        })
        body.addView(ui.group(
            ui.valueRow(t(R.string.about_source), "GitHub") { host.openUrl(SOURCE_URL) },
            ui.valueRow(t(R.string.about_license), t(R.string.about_license_value)) { host.openUrl("$SOURCE_URL/blob/main/LICENSE") },
        ))
        body.addView(ui.card(12).apply {
            addView(ui.sectionLabel(t(R.string.about_third)))
            THIRD_PARTY.forEach { (name, license) ->
                addView(ui.box(false).apply {
                    addView(ui.text(name, 14f), ui.lp(0, Ui.WRAP, 1f))
                    addView(ui.text(license, 14f, Pal.MUTED))
                })
            }
            addView(ui.outlineButton(t(R.string.about_all)) {
                AlertDialog.Builder(act)
                    .setTitle(t(R.string.about_third))
                    .setMessage(t(R.string.about_all_msg))
                    .setPositiveButton(t(R.string.ok), null)
                    .show()
            })
        })
        body.addView(ui.text(t(R.string.about_disclaimer), 12f, Pal.FAINT).apply {
            gravity = Gravity.CENTER
            setPadding(ui.dp(8), ui.dp(4), ui.dp(8), 0)
        }, ui.lp())
    }

    private fun languageName(tag: String) =
        if (tag.isEmpty()) t(R.string.language_auto_short) else L10n.OPTIONS.first { it.first == tag }.second

    private companion object {
        const val SOURCE_URL = "https://github.com/Thallanar/GBAEmu"
        val THIRD_PARTY = listOf(
            "ScaleFX (Sp00kyFox)" to "MIT",
            "HQx" to "LGPL-2.1",
            "xBRZ" to "GPL-3.0",
        )
    }
}
