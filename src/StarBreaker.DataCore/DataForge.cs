﻿using System.IO.Enumeration;
using System.Text;
using System.Xml;

namespace StarBreaker.DataCore;

public class DataForge
{
    public DataCoreBinary DataCore { get; }

    public DataForge(Stream stream)
    {
        DataCore = new DataCoreBinary(stream);
    }

    public Dictionary<string, string[]> ExportEnums()
    {
        var result = new Dictionary<string, string[]>(DataCore.Database.EnumDefinitions.Length);

        foreach (var enumDef in DataCore.Database.EnumDefinitions)
        {
            var enumValues = new string[enumDef.ValueCount];
            for (var i = 0; i < enumDef.ValueCount; i++)
            {
                enumValues[i] = DataCore.Database.GetString2(DataCore.Database.EnumOptions[enumDef.FirstValueIndex + i]);
            }

            result.Add(enumDef.GetName(DataCore.Database), enumValues);
        }

        return result;
    }

    public void ExtractAll(string outputFolder, string? fileNameFilter = null, IProgress<double>? progress = null)
    {
        var progressValue = 0;
        var recordsByFileName = DataCore.GetRecordsByFileName(fileNameFilter);
        var total = recordsByFileName.Count;

        foreach (var (fileName, record) in recordsByFileName)
        {
            var filePath = Path.Combine(outputFolder, fileName);

            Directory.CreateDirectory(Path.GetDirectoryName(filePath)!);

            {
                using var writer = new StreamWriter(filePath);

                ExtractSingleRecord(writer, record);
            }

            var currentProgress = Interlocked.Increment(ref progressValue);
            //only report progress every 250 records and when we are done
            if (currentProgress == total || currentProgress % 250 == 0)
                progress?.Report(currentProgress / (double)total);
        }
    }

    public void ExtractSingleRecord(TextWriter writer, DataCoreRecord record)
    {
        var node = DataCore.GetFromRecord(record);

        using var xmlWriter = XmlWriter.Create(writer, new XmlWriterSettings { Indent = true });

        node.WriteTo(xmlWriter);
    }
}