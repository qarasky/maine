using System.Windows;

namespace Maine.App;

/// <summary>Small secure-input dialog: session rename and the askpass fallback.</summary>
public partial class PromptDialog : Window
{
    public string? Answer { get; private set; }

    public PromptDialog(string title, string prompt, string initial = "")
    {
        InitializeComponent();
        Title = title;
        PromptText.Text = prompt;
        AnswerBox.Password = initial;
        Loaded += (_, _) =>
        {
            AnswerBox.Focus();
            AnswerBox.SelectAll();
        };
    }

    private void OnOk(object sender, RoutedEventArgs e)
    {
        Answer = AnswerBox.Password;
        DialogResult = true;
    }
}
