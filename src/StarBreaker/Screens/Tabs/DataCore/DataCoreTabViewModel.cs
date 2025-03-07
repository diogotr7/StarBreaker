using Avalonia.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using StarBreaker.DataCore;
using StarBreaker.Services;

namespace StarBreaker.Screens;

public sealed partial class DataCoreTabViewModel : PageViewModelBase
{
    private const string dataCorePath = "Data\\Game2.dcb";
    public override string Name => "DataCore";
    public override string Icon => "ViewAll";

    private readonly IP4kService _p4KService;

    public DataCoreTabViewModel(IP4kService p4kService)
    {
        _p4KService = p4kService;
        DataCore = null;

        Task.Run(Initialize);
    }

    private void Initialize()
    {
        var entry = _p4KService.P4KFileSystem.OpenRead(dataCorePath);
        var dcb = new DataCoreBinaryXml(new DataCoreDatabase(entry));
        entry.Dispose();
        
        Dispatcher.UIThread.InvokeAsync(() => DataCore = dcb);
    }

    [ObservableProperty] 
    private DataCoreBinaryXml? _dataCore;

    public string Yes => DataCore?.Database.RecordDefinitions.Length.ToString() ?? "No";
}