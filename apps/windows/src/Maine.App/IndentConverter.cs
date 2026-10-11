using System.Globalization;
using System.Windows;
using System.Windows.Data;

namespace Maine.App;

/// <summary>Indentation level (0 or 1) to a left margin for child session rows.</summary>
public sealed class IndentToMarginConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, CultureInfo culture) =>
        new Thickness(System.Convert.ToInt32(value) * 16, 0, 0, 0);

    public object ConvertBack(object value, Type targetType, object parameter, CultureInfo culture) =>
        throw new NotSupportedException();
}
