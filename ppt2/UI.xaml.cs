using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace Zetris.PPT {
    public partial class UI: IUI {
        static bool FreezeEvents = true;

        /// One line per `Titan.Personalities` entry (src/bot.rs).
        static readonly string[] PersonalityInfos = {
            "All-round best play. Lower strength makes human-like mistakes.",
            "Showman: varied tricks and named setups when they're nearly free.",
            "Keeps the back-to-back chain alive whenever it can.",
            "Combo specialist: builds a 4-wide well and chains clears.",
            "Keeps the stack low and flat, goes for perfect clears.",
            "Clears only with tricks, never the same twice in a row. PPT2 scores only T-spins, so its other spins count as plain clears."
        };

        public UI() {
            InitializeComponent();

            FreezeEvents = false;

            int? gamepadIndex = null;

            if (Keyboard.IsKeyDown(Key.LeftShift) && (gamepadIndex = new GamepadIndexWindow().Ask()).HasValue)
                Title = $"Titan for PPT2 [{gamepadIndex}]";

            foreach (string name in Titan.Personalities)
                Personality.Items.Add(name);

            Personality.SelectedIndex = Preferences.Personality;
            Strength.RawValue = Preferences.Strength;
            Speed.RawValue = Preferences.Speed;
            HoldAllowed.IsChecked = Preferences.HoldAllowed;
            Player.RawValue = Preferences.Player + 1;
            AccurateSync.IsChecked = Preferences.AccurateSync;

            GamepadPanel.RestoreText = "Restore Inputs";
            Version.Text = App.Version;

            UpdatePersonality();
            UpdateActive();
            PPTBot.Instance.Start(this, gamepadIndex?? 4);
        }

        bool _active;
        public bool Active {
            get => _active;
            set {
                if (_active == value) return;

                _active = value;
                Dispatcher.InvokeAsync(UpdateActive);
            }
        }

        public void SetConfidence(string confidence) {
            Dispatcher.InvokeAsync(() => {
                Confidence.Text = $"Positions: {confidence}";
                Info.MaxHeight = double.PositiveInfinity;
            });
        }

        public void SetThinkingTime(long time) {
            Dispatcher.InvokeAsync(() => {
                ThinkingTime.Text = $"Thinking Time: {time}ms";
                Info.MaxHeight = double.PositiveInfinity;
            });
        }

        public void SetPlan(string plan) {
            Dispatcher.InvokeAsync(() => {
                Plan.Text = $"Plan: {plan}";
                Info.MaxHeight = double.PositiveInfinity;
            });
        }

        void UpdateActive() {
            State.Text = Active? "Active" : "Inactive";
            Personality.IsEnabled = Speed.Enabled = HoldAllowed.IsEnabled = Player.Enabled = !Active;
            Strength.Enabled = !Active && Preferences.Personality == 0;

            if (!Active) Info.MaxHeight = 0;
        }

        void UpdatePersonality() {
            PersonalityInfo.Text = PersonalityInfos[Preferences.Personality];
            Strength.Enabled = !Active && Preferences.Personality == 0;
        }

        void PersonalityChanged(object sender, SelectionChangedEventArgs e) {
            if (!FreezeEvents) Preferences.Personality = Personality.SelectedIndex;
            UpdatePersonality();
        }

        void StrengthChanged(Dial sender, double NewValue) {
            if (!FreezeEvents) Preferences.Strength = (int)Strength.RawValue;
        }

        void SpeedChanged(Dial sender, double NewValue) {
            if (!FreezeEvents) Preferences.Speed = (int)Speed.RawValue;
        }

        void HoldAllowedChanged(object sender, RoutedEventArgs e) {
            if (!FreezeEvents) Preferences.HoldAllowed = HoldAllowed.IsChecked == true;
        }

        void PlayerChanged(Dial sender, double NewValue) {
            if (!FreezeEvents) Preferences.Player = (int)Player.RawValue - 1;
        }

        void GamepadChanged(object sender, RoutedEventArgs e) {
            if (!FreezeEvents) {
                FreezeEvents = true;
                Gamepad.IsChecked = PPTBot.Instance.SetGamepad(Gamepad.IsChecked == true);
                FreezeEvents = false;
            }
        }

        void AccurateSyncChanged(object sender, RoutedEventArgs e) {
            if (!FreezeEvents) Preferences.AccurateSync = AccurateSync.IsChecked == true;
        }
    }
}
